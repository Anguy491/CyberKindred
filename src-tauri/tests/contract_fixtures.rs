use std::{error::Error, fs, path::PathBuf};

use cyberkindred_lib::contracts::{
    ContractRegistry, MemoryRecord, PlaybackEvent, PlaybackState, ProgramPlan, ProviderError,
    ScheduleRule,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

#[derive(Deserialize)]
struct ContractCase {
    schema: String,
    example: String,
    expected: bool,
}

fn repository_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}

fn round_trip<T>(
    registry: &ContractRegistry,
    schema: &str,
    document: &Value,
) -> Result<(), Box<dyn Error>>
where
    T: DeserializeOwned + Serialize,
{
    let typed: T = registry.deserialize(schema, document)?;
    let serialized = serde_json::to_value(typed)?;
    registry.validate(schema, &serialized)?;
    if !json_equivalent(&serialized, document) {
        return Err("typed contract round trip changed the document".into());
    }
    Ok(())
}

fn json_equivalent(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => left.as_f64() == right.as_f64(),
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| json_equivalent(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, value)| {
                    right
                        .get(key)
                        .is_some_and(|right| json_equivalent(value, right))
                })
        }
        _ => left == right,
    }
}

fn round_trip_for_schema(
    registry: &ContractRegistry,
    schema: &str,
    document: &Value,
) -> Result<(), Box<dyn Error>> {
    match schema {
        "memory-record" => round_trip::<MemoryRecord>(registry, schema, document),
        "playback-event" => round_trip::<PlaybackEvent>(registry, schema, document),
        "playback-state" => round_trip::<PlaybackState>(registry, schema, document),
        "program-plan" => round_trip::<ProgramPlan>(registry, schema, document),
        "provider-error" => round_trip::<ProviderError>(registry, schema, document),
        "schedule-rule" => round_trip::<ScheduleRule>(registry, schema, document),
        _ => Err("unknown test schema".into()),
    }
}

#[test]
fn contract_fixtures_match_schema_and_typed_models() -> Result<(), Box<dyn Error>> {
    let registry = ContractRegistry::new()?;
    let cases_source = fs::read_to_string(repository_path("tests/contracts/cases.json"))?;
    let cases: Vec<ContractCase> = serde_json::from_str(&cases_source)?;

    for case in cases {
        let source = fs::read_to_string(repository_path(&format!(
            "docs/contracts/examples/{}",
            case.example
        )))?;
        let document = serde_json::from_str::<Value>(&source)?;
        let result = registry.validate(&case.schema, &document);
        if result.is_ok() != case.expected {
            return Err(format!("unexpected validation result for {}", case.example).into());
        }
        if case.expected {
            round_trip_for_schema(&registry, &case.schema, &document).map_err(|error| {
                format!("typed round trip failed for {}: {error}", case.example)
            })?;
        }
    }
    Ok(())
}

#[test]
fn contract_validation_errors_do_not_echo_payloads() -> Result<(), Box<dyn Error>> {
    let registry = ContractRegistry::new()?;
    let canary = "contract-secret-canary";
    let document = serde_json::json!({ "unexpected": canary });
    let error = registry
        .validate("memory-record", &document)
        .err()
        .ok_or("invalid document unexpectedly passed")?;
    if error.to_string().contains(canary) {
        return Err("contract error exposed submitted content".into());
    }
    Ok(())
}
