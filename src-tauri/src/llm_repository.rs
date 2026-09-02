//! Current-setting and origin-scoped credential adapter for program planning.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{
    llm::{OpenAiProgramCredential, ProgramCredentialSource},
    program::{ProgramFuture, ProgramProviderError},
    storage::{CanonicalOrigin, CredentialTarget, Repository, SecretVault},
};

pub(crate) struct RepositoryProgramCredentialSource {
    repository: Repository,
    vault: Arc<Mutex<Box<dyn SecretVault>>>,
}

impl RepositoryProgramCredentialSource {
    pub(crate) fn new(repository: Repository, vault: Box<dyn SecretVault>) -> Self {
        Self {
            repository,
            vault: Arc::new(Mutex::new(vault)),
        }
    }
}

impl ProgramCredentialSource for RepositoryProgramCredentialSource {
    fn load(&self) -> ProgramFuture<'_, Result<OpenAiProgramCredential, ProgramProviderError>> {
        Box::pin(async move {
            let settings = self
                .repository
                .load_provider_settings()
                .await
                .map_err(|_| ProgramProviderError::Unavailable)?;
            let origin = CanonicalOrigin::parse(&settings.provider_origin)
                .map_err(|_| ProgramProviderError::InvalidResponse)?;
            let target = CredentialTarget::openai(&origin);
            let secret = self
                .vault
                .lock()
                .await
                .get(&target)
                .map_err(|_| ProgramProviderError::Unavailable)?
                .ok_or(ProgramProviderError::Unavailable)?;
            OpenAiProgramCredential::new(origin, settings.llm_model_id, secret)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::storage::{AppPaths, SecretError, SecretValue, Storage};

    #[derive(Default)]
    struct FakeVault(HashMap<String, String>);

    impl SecretVault for FakeVault {
        fn set(
            &mut self,
            target: &CredentialTarget,
            value: SecretValue,
        ) -> Result<(), SecretError> {
            let value = value.with_exposed(ToOwned::to_owned);
            self.0.insert(target.as_resource().to_owned(), value);
            Ok(())
        }

        fn get(&self, target: &CredentialTarget) -> Result<Option<SecretValue>, SecretError> {
            self.0
                .get(target.as_resource())
                .map(|value| SecretValue::new(value.clone()))
                .transpose()
        }

        fn delete(&mut self, target: &CredentialTarget) -> Result<(), SecretError> {
            self.0.remove(target.as_resource());
            Ok(())
        }

        fn delete_cyberkindred_namespace(&mut self) -> Result<u32, SecretError> {
            let count = u32::try_from(self.0.len()).map_err(|_| SecretError::OperationFailed)?;
            self.0.clear();
            Ok(count)
        }
    }

    #[tokio::test]
    async fn llm_credential_source_loads_only_the_current_origin_target() {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("app paths");
        let storage = Storage::open(&paths, "0.3.0").await.expect("storage");
        let repository = storage.repository();
        let origin = CanonicalOrigin::parse("https://api.openai.com").expect("origin");
        let mut vault = FakeVault::default();
        vault
            .set(
                &CredentialTarget::openai(&origin),
                SecretValue::new("fixture-secret-never-logged".to_owned()).expect("secret"),
            )
            .expect("vault write");
        let source = RepositoryProgramCredentialSource::new(repository, Box::new(vault));
        source.load().await.expect("credential");
        storage.close().await;
    }
}
