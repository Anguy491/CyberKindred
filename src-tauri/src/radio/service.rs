use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    ipc::{ApiError, InternalReason, ProcessSequence, PublicField, canonical_request_hash},
    program::{LocalPlanExpectation, ProgramError, validate_local_program_plan},
};

use super::{
    AppleCompanion, AppleCompanionSignal, ProgramAck, ProgramEventState, ProgramPlayback,
    ProgramRadioPlanner, ProgramRunPhase, ProgramSpeech, ProgramStartAuthorizer, RadioClock,
    RadioEventSink, RadioProgramStore, StartProgramRequest, StartProgramResponse,
    StopProgramRequest,
    apple::disconnected_message,
    events::RadioEventPublisher,
    idempotency::AsyncIdempotency,
    runner::{ActiveRun, ProgramRunner},
};

const IDEMPOTENCY_CAPACITY: usize = 256;
const TERMINAL_CAPACITY: usize = 256;
const LOCAL_SOURCE_ID: &str = "local";
const APPLE_SOURCE_ID: &str = "apple_music";
const PLANNING_FAILED_MESSAGE: &str = "节目计划生成失败，未开始播放。";
const COMPANION_RUNNING_MESSAGE: &str =
    "COMPANION MODE：队列由 Apple Music 控制；曲目反应在本机生成。";
const COMPANION_FAILED_MESSAGE: &str = "Apple Music 陪伴模式已安全停止；未改动外部队列。";

pub trait RadioIdFactory: Send + Sync {
    fn next_id(&self) -> Uuid;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRadioIdFactory;

impl RadioIdFactory for SystemRadioIdFactory {
    fn next_id(&self) -> Uuid {
        Uuid::now_v7()
    }
}

/// Explicit production/test composition boundary. Construction performs no
/// planning, provider, speech, persistence, or audio work.
pub struct RadioServiceDependencies {
    pub planner: Arc<dyn ProgramRadioPlanner>,
    pub authorizer: Arc<dyn ProgramStartAuthorizer>,
    pub store: Arc<dyn RadioProgramStore>,
    pub playback: Arc<dyn ProgramPlayback>,
    pub speech: Arc<dyn ProgramSpeech>,
    pub apple: Arc<dyn AppleCompanion>,
    pub event_sink: Arc<dyn RadioEventSink>,
    pub clock: Arc<dyn RadioClock>,
    pub sequence: Arc<ProcessSequence>,
    pub id_factory: Arc<dyn RadioIdFactory>,
}

struct ServiceState {
    active: Option<Arc<ActiveRun>>,
    terminal_revisions: HashMap<Uuid, u64>,
    terminal_order: VecDeque<Uuid>,
}

impl ServiceState {
    fn new() -> Self {
        Self {
            active: None,
            terminal_revisions: HashMap::with_capacity(TERMINAL_CAPACITY),
            terminal_order: VecDeque::with_capacity(TERMINAL_CAPACITY),
        }
    }

    fn record_terminal(&mut self, program_id: Uuid, revision: u64) {
        if !self.terminal_revisions.contains_key(&program_id) {
            if self.terminal_order.len() >= TERMINAL_CAPACITY
                && let Some(oldest) = self.terminal_order.pop_front()
            {
                self.terminal_revisions.remove(&oldest);
            }
            self.terminal_order.push_back(program_id);
        }
        self.terminal_revisions.insert(program_id, revision);
    }
}

struct RadioServiceInner {
    planner: Arc<dyn ProgramRadioPlanner>,
    authorizer: Arc<dyn ProgramStartAuthorizer>,
    store: Arc<dyn RadioProgramStore>,
    runner: ProgramRunner,
    apple: Arc<dyn AppleCompanion>,
    events: RadioEventPublisher,
    clock: Arc<dyn RadioClock>,
    id_factory: Arc<dyn RadioIdFactory>,
    state: Mutex<ServiceState>,
    start_requests: AsyncIdempotency<StartProgramResponse>,
    stop_requests: AsyncIdempotency<ProgramAck>,
}

#[derive(Clone)]
pub struct RadioService {
    inner: Arc<RadioServiceInner>,
}

impl RadioService {
    #[must_use]
    pub fn new(dependencies: RadioServiceDependencies) -> Self {
        let events = RadioEventPublisher::new(
            Arc::clone(&dependencies.event_sink),
            Arc::clone(&dependencies.sequence),
            Arc::clone(&dependencies.clock),
        );
        let runner = ProgramRunner::new(
            Arc::clone(&dependencies.store),
            dependencies.playback,
            dependencies.speech,
            events.clone(),
            Arc::clone(&dependencies.clock),
        );
        Self {
            inner: Arc::new(RadioServiceInner {
                planner: dependencies.planner,
                authorizer: dependencies.authorizer,
                store: dependencies.store,
                runner,
                apple: dependencies.apple,
                events,
                clock: dependencies.clock,
                id_factory: dependencies.id_factory,
                state: Mutex::new(ServiceState::new()),
                start_requests: AsyncIdempotency::new(IDEMPOTENCY_CAPACITY),
                stop_requests: AsyncIdempotency::new(IDEMPOTENCY_CAPACITY),
            }),
        }
    }

    /// Starts API-024 only after proving user authority, then persists identity
    /// and the complete validated plan before spawning the sound-capable runner.
    ///
    /// # Errors
    ///
    /// Returns stable validation, authority, busy, planning, or storage errors.
    pub async fn start_local_program(
        &self,
        request: StartProgramRequest,
    ) -> Result<StartProgramResponse, ApiError> {
        validate_start_request(&request)?;
        let request_hash = canonical_request_hash(&request)?;
        self.inner
            .start_requests
            .execute(request.client_request_id, request_hash, || {
                self.start_once(request)
            })
            .await
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the confirmed start, durable reservation, and source-specific handoff remain one auditable transaction"
    )]
    async fn start_once(
        &self,
        request: StartProgramRequest,
    ) -> Result<StartProgramResponse, ApiError> {
        let authorization = self.inner.authorizer.authorize(&request).await?;
        let program_id = valid_generated_id(self.inner.id_factory.next_id())?;
        let active = ActiveRun::new(program_id);
        self.reserve(Arc::clone(&active)).await?;

        let apple_initial = if request.source_id == APPLE_SOURCE_ID {
            match self.inner.apple.connect().await {
                Ok(state) => Some(state),
                Err(error) => {
                    self.release_unstarted(program_id).await;
                    return Err(error);
                }
            }
        } else {
            None
        };

        let begin = if apple_initial.is_some() {
            self.inner
                .store
                .begin_system_program(program_id, self.inner.clock.now_ms())
                .await
        } else {
            self.inner
                .store
                .begin_program(program_id, self.inner.clock.now_ms())
                .await
        };
        if let Err(error) = begin {
            self.release_unstarted(program_id).await;
            return Err(error);
        }
        self.inner
            .events
            .state(program_id, ProgramEventState::Planning, None);

        if let Some(initial) = apple_initial {
            let revision = match self
                .inner
                .store
                .transition_program(
                    program_id,
                    ProgramRunPhase::Planning,
                    ProgramRunPhase::Ready,
                    0,
                    self.inner.clock.now_ms(),
                    None,
                )
                .await
            {
                Ok(revision) => revision,
                Err(error) => {
                    self.release_unstarted(program_id).await;
                    return Err(error);
                }
            };
            let inner = Arc::clone(&self.inner);
            tokio::spawn(async move {
                let result = inner
                    .run_apple(Arc::clone(&active), initial, authorization, revision)
                    .await;
                inner.finish_run(active, result).await;
            });
            return Ok(StartProgramResponse {
                request_id: request.client_request_id,
                program_id,
                plan: None,
            });
        }

        let planned = match self.inner.planner.plan_local(program_id).await {
            Ok(planned) => planned,
            Err(error) => {
                self.fail_before_plan(program_id, 0, "planning_failed")
                    .await?;
                return Err(error);
            }
        };
        if validate_planned(program_id, &planned).is_err() {
            self.fail_before_plan(program_id, 0, "invalid_plan").await?;
            return Err(ApiError::from_reason(
                InternalReason::ProviderInvalidResponse,
            ));
        }
        if let Err(error) = self.inner.store.persist_plan(&planned).await {
            self.fail_before_plan(program_id, 0, "plan_persist_failed")
                .await?;
            return Err(error);
        }

        let plan = planned.plan.clone();
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let result = inner
                .runner
                .run(Arc::clone(&active), planned, authorization, 1)
                .await;
            inner.finish_run(active, result).await;
        });
        Ok(StartProgramResponse {
            request_id: request.client_request_id,
            program_id,
            plan: Some(plan),
        })
    }

    /// Idempotently stops API-025 and waits until the runner has persisted its
    /// terminal state and cancelled current speech/playback work.
    ///
    /// # Errors
    ///
    /// Returns stable validation, not-found, idempotency, or terminal errors.
    pub async fn stop_program(&self, request: StopProgramRequest) -> Result<ProgramAck, ApiError> {
        validate_stop_request(&request)?;
        let request_hash = canonical_request_hash(&request)?;
        self.inner
            .stop_requests
            .execute(request.client_request_id, request_hash, || {
                self.stop_once(request)
            })
            .await
    }

    async fn stop_once(&self, request: StopProgramRequest) -> Result<ProgramAck, ApiError> {
        let active = {
            let state = self.inner.state.lock().await;
            if let Some(revision) = state.terminal_revisions.get(&request.program_id) {
                return Ok(ProgramAck {
                    request_id: request.client_request_id,
                    revision: *revision,
                });
            }
            state
                .active
                .as_ref()
                .filter(|active| active.program_id == request.program_id)
                .cloned()
                .ok_or_else(|| ApiError::from_reason(InternalReason::EntityNotFound))?
        };
        active.cancel();
        let revision = active.wait().await?;
        Ok(ProgramAck {
            request_id: request.client_request_id,
            revision,
        })
    }

    async fn reserve(&self, active: Arc<ActiveRun>) -> Result<(), ApiError> {
        let mut state = self.inner.state.lock().await;
        if state.active.is_some() {
            return Err(ApiError::from_reason(
                InternalReason::OperationAlreadyRunning,
            ));
        }
        state.active = Some(active);
        Ok(())
    }

    async fn release_unstarted(&self, program_id: Uuid) {
        let mut state = self.inner.state.lock().await;
        if state.active.as_ref().map(|active| active.program_id) == Some(program_id) {
            state.active = None;
        }
    }

    async fn fail_before_plan(
        &self,
        program_id: Uuid,
        revision: u64,
        failure_code: &'static str,
    ) -> Result<(), ApiError> {
        let terminal = self
            .inner
            .store
            .transition_program(
                program_id,
                ProgramRunPhase::Planning,
                ProgramRunPhase::Failed,
                revision,
                self.inner.clock.now_ms(),
                Some(failure_code),
            )
            .await;
        let revision = match terminal {
            Ok(revision) => revision,
            Err(error) => {
                self.release_unstarted(program_id).await;
                return Err(error);
            }
        };
        self.inner.events.state(
            program_id,
            ProgramEventState::Failed,
            Some(PLANNING_FAILED_MESSAGE),
        );
        let mut state = self.inner.state.lock().await;
        state.active = None;
        state.record_terminal(program_id, revision);
        Ok(())
    }
}

impl RadioServiceInner {
    async fn run_apple(
        &self,
        active: Arc<ActiveRun>,
        initial: crate::contracts::PlaybackState,
        authorization: super::ConfirmedProgramStart,
        revision: u64,
    ) -> Result<u64, ApiError> {
        let mut revision = self
            .store
            .transition_program(
                active.program_id,
                ProgramRunPhase::Ready,
                ProgramRunPhase::Music,
                revision,
                self.clock.now_ms(),
                None,
            )
            .await?;
        self.events.state(
            active.program_id,
            ProgramEventState::Running,
            Some(COMPANION_RUNNING_MESSAGE),
        );
        let (sender, mut receiver) = tokio::sync::mpsc::channel(16);
        let monitor = self.apple.run(
            active.program_id,
            initial,
            authorization,
            active.cancellation(),
            sender,
        );
        tokio::pin!(monitor);
        loop {
            tokio::select! {
                result = &mut monitor => {
                    if result.is_err() && !*active.cancellation().borrow() {
                        revision = self.store.transition_program(
                            active.program_id,
                            ProgramRunPhase::Music,
                            ProgramRunPhase::Failed,
                            revision,
                            self.clock.now_ms(),
                            Some("apple_monitor_failed"),
                        ).await?;
                        self.events.state(
                            active.program_id,
                            ProgramEventState::Failed,
                            Some(COMPANION_FAILED_MESSAGE),
                        );
                        return Ok(revision);
                    }
                    revision = self.store.transition_program(
                        active.program_id,
                        ProgramRunPhase::Music,
                        ProgramRunPhase::Stopping,
                        revision,
                        self.clock.now_ms(),
                        Some("user_stop"),
                    ).await?;
                    self.events.state(active.program_id, ProgramEventState::Stopping, None);
                    revision = self.store.transition_program(
                        active.program_id,
                        ProgramRunPhase::Stopping,
                        ProgramRunPhase::Completed,
                        revision,
                        self.clock.now_ms(),
                        Some("user_stop"),
                    ).await?;
                    self.events.state(active.program_id, ProgramEventState::Completed, None);
                    return Ok(revision);
                }
                signal = receiver.recv() => match signal {
                    Some(AppleCompanionSignal::Reaction(message)) => {
                        self.events.state(
                            active.program_id,
                            ProgramEventState::Running,
                            Some(&message),
                        );
                    }
                    Some(AppleCompanionSignal::Disconnected) => {
                        self.events.state(
                            active.program_id,
                            ProgramEventState::Paused,
                            Some(disconnected_message()),
                        );
                    }
                    None => {}
                }
            }
        }
    }

    async fn finish_run(&self, active: Arc<ActiveRun>, result: Result<u64, ApiError>) {
        {
            let mut state = self.state.lock().await;
            if state.active.as_ref().map(|current| current.program_id) == Some(active.program_id) {
                state.active = None;
            }
            if let Ok(revision) = result {
                state.record_terminal(active.program_id, revision);
            }
        }
        active.finish(result).await;
    }
}

fn validate_start_request(request: &StartProgramRequest) -> Result<(), ApiError> {
    if request.client_request_id.get_version_num() != 7
        || !matches!(
            request.source_id.as_str(),
            LOCAL_SOURCE_ID | APPLE_SOURCE_ID
        )
    {
        return Err(
            ApiError::from_reason(InternalReason::RequestInvalid).with_field(PublicField::Request)
        );
    }
    Ok(())
}

fn validate_stop_request(request: &StopProgramRequest) -> Result<(), ApiError> {
    if request.client_request_id.get_version_num() != 7 || request.program_id.get_version_num() != 7
    {
        return Err(
            ApiError::from_reason(InternalReason::RequestInvalid).with_field(PublicField::Request)
        );
    }
    Ok(())
}

fn valid_generated_id(program_id: Uuid) -> Result<Uuid, ApiError> {
    if program_id.get_version_num() == 7 {
        Ok(program_id)
    } else {
        Err(ApiError::unexpected())
    }
}

fn validate_planned(
    program_id: Uuid,
    planned: &crate::program::PlannedProgram,
) -> Result<(), ProgramError> {
    let candidate_track_ids = planned
        .candidates
        .candidates
        .iter()
        .map(|candidate| candidate.track_id.clone())
        .collect::<HashSet<_>>();
    validate_local_program_plan(
        &planned.plan,
        &LocalPlanExpectation {
            program_id,
            source_id: LOCAL_SOURCE_ID,
            created_at: &planned.plan.created_at,
            candidate_track_ids: &candidate_track_ids,
        },
    )
}
