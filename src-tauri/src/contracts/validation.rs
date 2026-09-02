use std::collections::HashMap;

use jsonschema::{Draft, Registry, Validator};
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;

const SCHEMA_SOURCES: [(&str, &str); 6] = [
    (
        "memory-record",
        include_str!("../../../docs/contracts/schemas/memory-record.schema.json"),
    ),
    (
        "playback-event",
        include_str!("../../../docs/contracts/schemas/playback-event.schema.json"),
    ),
    (
        "playback-state",
        include_str!("../../../docs/contracts/schemas/playback-state.schema.json"),
    ),
    (
        "program-plan",
        include_str!("../../../docs/contracts/schemas/program-plan.schema.json"),
    ),
    (
        "provider-error",
        include_str!("../../../docs/contracts/schemas/provider-error.schema.json"),
    ),
    (
        "schedule-rule",
        include_str!("../../../docs/contracts/schemas/schedule-rule.schema.json"),
    ),
];

/// Safe contract-validation failures that never include the submitted document.
#[derive(Debug, Error)]
pub enum ContractError {
    #[error("contract schema source is invalid")]
    InvalidSchemaSource,
    #[error("contract schema registry could not be built")]
    RegistryBuild,
    #[error("unknown contract schema")]
    UnknownSchema,
    #[error("contract document failed {violations} validation rule(s)")]
    ValidationFailed { violations: usize },
    #[error("validated contract document could not be deserialized")]
    DeserializationFailed,
}

/// Offline registry for the six public v1 JSON Schemas.
pub struct ContractRegistry {
    validators: HashMap<&'static str, Validator>,
}

impl ContractRegistry {
    /// Compiles every schema as Draft 2020-12 with format validation and local refs.
    ///
    /// # Errors
    ///
    /// Returns a safe error if a checked-in schema or its local registry is invalid.
    pub fn new() -> Result<Self, ContractError> {
        let mut schemas = Vec::with_capacity(SCHEMA_SOURCES.len());
        for (name, source) in SCHEMA_SOURCES {
            let schema = serde_json::from_str::<Value>(source)
                .map_err(|_| ContractError::InvalidSchemaSource)?;
            schemas.push((name, schema));
        }

        let mut registry_builder = Registry::new();
        for (_, schema) in &schemas {
            let id = schema
                .get("$id")
                .and_then(Value::as_str)
                .ok_or(ContractError::InvalidSchemaSource)?;
            registry_builder = registry_builder
                .add(id, schema)
                .map_err(|_| ContractError::RegistryBuild)?;
        }
        let registry = registry_builder
            .prepare()
            .map_err(|_| ContractError::RegistryBuild)?;

        let mut validators = HashMap::with_capacity(schemas.len());
        for (name, schema) in &schemas {
            let validator = jsonschema::options()
                .with_draft(Draft::Draft202012)
                .should_validate_formats(true)
                .should_ignore_unknown_formats(false)
                .with_registry(&registry)
                .build(schema)
                .map_err(|_| ContractError::RegistryBuild)?;
            validators.insert(*name, validator);
        }
        Ok(Self { validators })
    }

    /// Validates an untrusted JSON value before any typed deserialization or side effect.
    ///
    /// # Errors
    ///
    /// Returns an unknown-schema or payload-safe validation error.
    pub fn validate(&self, schema: &str, document: &Value) -> Result<(), ContractError> {
        let validator = self
            .validators
            .get(schema)
            .ok_or(ContractError::UnknownSchema)?;
        let violations = validator.iter_errors(document).count();
        if violations == 0 {
            Ok(())
        } else {
            Err(ContractError::ValidationFailed { violations })
        }
    }

    /// Validates and then deserializes a public contract document.
    ///
    /// # Errors
    ///
    /// Returns a payload-safe contract error; submitted values are never formatted into it.
    pub fn deserialize<T: DeserializeOwned>(
        &self,
        schema: &str,
        document: &Value,
    ) -> Result<T, ContractError> {
        self.validate(schema, document)?;
        serde_json::from_value(document.clone()).map_err(|_| ContractError::DeserializationFailed)
    }
}
