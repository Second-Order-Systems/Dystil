//! Provider-agnostic inference resolution for Dystil product features.
//!
//! Harness-specific details stop here. Callers receive only `dyn AiRuntime`
//! and normalized descriptor/error types from `dystil-ai`.

use async_trait::async_trait;
use tauri::AppHandle;

use dystil_ai::{
    AiAnswerRequest, AiAutomationRequest, AiAutomationRun, AiModelTier, AiRuntime,
    AiRuntimeDescriptor, AiRuntimeError, AiRuntimeErrorCode, AiRuntimeEvent, AiRuntimeKind,
    AiStructuredRequest, AiStructuredRun, CliProvider, ProviderKind, TeammateAnswerRun,
};

use crate::{ai, ai_presets, recording::RecordingState};

struct CliRuntimeAdapter {
    descriptor: AiRuntimeDescriptor,
    runtime: CliProvider,
    model: Option<String>,
}

#[async_trait]
impl AiRuntime for CliRuntimeAdapter {
    fn descriptor(&self) -> &AiRuntimeDescriptor {
        &self.descriptor
    }

    fn model_for_tier(&self, tier: AiModelTier) -> String {
        managed_structured_model(self.descriptor.kind.clone(), tier).into()
    }

    async fn answer(&self, request: AiAnswerRequest) -> Result<TeammateAnswerRun, AiRuntimeError> {
        self.runtime
            .run_teammate_answer_with_model(
                &request.requester_name,
                &request.question,
                &request.search_start,
                &request.search_end,
                &request.timezone,
                self.model.as_deref(),
            )
            .await
            .map_err(Into::into)
    }

    async fn run_automation(
        &self,
        request: AiAutomationRequest,
        events: tokio::sync::mpsc::Sender<AiRuntimeEvent>,
    ) -> Result<AiAutomationRun, AiRuntimeError> {
        self.runtime
            .run_automation_with_model(request, self.model.as_deref(), events)
            .await
            .map_err(Into::into)
    }

    async fn infer_structured(
        &self,
        request: AiStructuredRequest,
    ) -> Result<AiStructuredRun, AiRuntimeError> {
        let model = self.model_for_tier(request.model_tier);
        tracing::info!(
            purpose = %request.purpose,
            model_tier = ?request.model_tier,
            model,
            provider = %self.descriptor.provider_label,
            "resolved structured inference model tier"
        );
        self.runtime
            .run_structured_with_model(request, Some(&model))
            .await
            .map_err(Into::into)
    }
}

fn managed_structured_model(kind: AiRuntimeKind, tier: AiModelTier) -> &'static str {
    match (kind, tier) {
        (AiRuntimeKind::Codex, AiModelTier::Economy) => "gpt-5.6-luna",
        (AiRuntimeKind::Codex, AiModelTier::Frontier) => "gpt-5.6-sol",
        (AiRuntimeKind::Claude, AiModelTier::Economy) => "claude-haiku-4-5",
        (AiRuntimeKind::Claude, AiModelTier::Frontier) => "opus",
        (AiRuntimeKind::Pi, _) => unreachable!("Pi does not use the managed CLI adapter"),
    }
}

struct PiRuntimeAdapter {
    descriptor: AiRuntimeDescriptor,
    preset: ai_presets::ActiveAiPreset,
    mcp: dystil_ai::McpServerConfig,
}

#[async_trait]
impl AiRuntime for PiRuntimeAdapter {
    fn descriptor(&self) -> &AiRuntimeDescriptor {
        &self.descriptor
    }

    fn model_for_tier(&self, tier: AiModelTier) -> String {
        api_model_for_tier(&self.preset.provider_kind, &self.preset.model, tier)
    }

    async fn answer(&self, request: AiAnswerRequest) -> Result<TeammateAnswerRun, AiRuntimeError> {
        ai_presets::pi_answer(&self.preset, &self.mcp, &request)
            .await
            .map_err(normalize_pi_error)
    }

    async fn run_automation(
        &self,
        request: AiAutomationRequest,
        events: tokio::sync::mpsc::Sender<AiRuntimeEvent>,
    ) -> Result<AiAutomationRun, AiRuntimeError> {
        ai_presets::pi_automation(&self.preset, &self.mcp, request, events)
            .await
            .map_err(normalize_pi_error)
    }

    async fn infer_structured(
        &self,
        request: AiStructuredRequest,
    ) -> Result<AiStructuredRun, AiRuntimeError> {
        let mut preset = self.preset.clone();
        preset.model = self.model_for_tier(request.model_tier);
        ai_presets::pi_structured(&preset, request)
            .await
            .map_err(normalize_pi_error)
    }
}

fn api_model_for_tier(provider: &str, configured: &str, tier: AiModelTier) -> String {
    match provider {
        "anthropic" => managed_structured_model(AiRuntimeKind::Claude, tier).into(),
        "openai" | "dystil_ai" => managed_structured_model(AiRuntimeKind::Codex, tier).into(),
        _ => configured.to_string(),
    }
}

pub(crate) async fn resolve(
    app: &AppHandle,
    state: &RecordingState,
    pool: &sqlx::SqlitePool,
    timezone: &str,
) -> Result<Box<dyn AiRuntime>, AiRuntimeError> {
    if let Some(preset) = ai_presets::active(pool)
        .await
        .map_err(|error| AiRuntimeError::new(AiRuntimeErrorCode::Internal, error))?
    {
        if matches!(
            preset.provider_kind.as_str(),
            "ollama" | "anthropic" | "openai" | "openai_compatible" | "dystil_ai"
        ) {
            if preset.provider_kind != "ollama" && preset.api_key.is_none() {
                return Err(AiRuntimeError::new(
                    AiRuntimeErrorCode::Authentication,
                    "the active AI preset has no credential in the operating-system keyring",
                ));
            }
            return Ok(Box::new(PiRuntimeAdapter {
                descriptor: AiRuntimeDescriptor {
                    kind: AiRuntimeKind::Pi,
                    provider_label: preset.name.clone(),
                    model: preset.model.clone(),
                },
                preset,
                mcp: ai::internal_mcp_server(app, state, timezone)
                    .await
                    .map_err(|error| AiRuntimeError::new(AiRuntimeErrorCode::NotReady, error))?,
            }));
        }
        let provider_name = preset.provider_kind.clone();
        let selected_model = preset.model.clone();
        return resolve_managed(app, state, timezone, &provider_name, &selected_model).await;
    }

    Err(AiRuntimeError::new(
        AiRuntimeErrorCode::NotReady,
        "no active AI preset; choose one in Settings",
    ))
}

async fn resolve_managed(
    app: &AppHandle,
    state: &RecordingState,
    timezone: &str,
    provider_name: &str,
    selected_model: &str,
) -> Result<Box<dyn AiRuntime>, AiRuntimeError> {
    let provider = ai::provider_kind(provider_name)
        .map_err(|error| AiRuntimeError::new(AiRuntimeErrorCode::NotReady, error))?;
    let runtime = ai::provider_runtime(provider.clone())
        .map_err(|error| AiRuntimeError::new(AiRuntimeErrorCode::NotReady, error))?;
    match runtime.authenticated().await {
        Ok(true) => {}
        Ok(false) => {
            return Err(AiRuntimeError::new(
                AiRuntimeErrorCode::Authentication,
                "configured AI provider is not authenticated",
            ))
        }
        Err(error) => return Err(AiRuntimeError::from(error)),
    }
    let runtime = runtime.with_mcp_server(
        ai::internal_mcp_server(app, state, timezone)
            .await
            .map_err(|error| AiRuntimeError::new(AiRuntimeErrorCode::NotReady, error))?,
    );
    let model = (selected_model != "default").then_some(selected_model.to_string());
    let kind = match provider {
        ProviderKind::Codex => AiRuntimeKind::Codex,
        ProviderKind::Claude => AiRuntimeKind::Claude,
    };
    Ok(Box::new(CliRuntimeAdapter {
        descriptor: AiRuntimeDescriptor {
            kind,
            provider_label: provider_name.to_string(),
            model: model.clone().unwrap_or_else(|| "default".into()),
        },
        runtime,
        model,
    }))
}

fn normalize_pi_error(error: String) -> AiRuntimeError {
    let lower = error.to_ascii_lowercase();
    let code = if lower.contains("not installed") || lower.contains("not reachable") {
        AiRuntimeErrorCode::NotReady
    } else if lower.contains("timed out") || lower.contains("timeout") {
        AiRuntimeErrorCode::Timeout
    } else if lower.contains("invalid") || lower.contains("json") {
        AiRuntimeErrorCode::InvalidOutput
    } else if lower.contains("401") || lower.contains("403") || lower.contains("key") {
        AiRuntimeErrorCode::Authentication
    } else {
        AiRuntimeErrorCode::Transport
    };
    AiRuntimeError::new(code, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_structured_tiers_map_to_provider_models() {
        assert_eq!(
            managed_structured_model(AiRuntimeKind::Codex, AiModelTier::Economy),
            "gpt-5.6-luna"
        );
        assert_eq!(
            managed_structured_model(AiRuntimeKind::Codex, AiModelTier::Frontier),
            "gpt-5.6-sol"
        );
        assert_eq!(
            managed_structured_model(AiRuntimeKind::Claude, AiModelTier::Economy),
            "claude-haiku-4-5"
        );
        assert_eq!(
            managed_structured_model(AiRuntimeKind::Claude, AiModelTier::Frontier),
            "opus"
        );
    }

    #[test]
    fn native_api_presets_share_managed_model_policy() {
        assert_eq!(
            api_model_for_tier("anthropic", "ignored", AiModelTier::Economy),
            "claude-haiku-4-5"
        );
        assert_eq!(
            api_model_for_tier("openai", "ignored", AiModelTier::Frontier),
            "gpt-5.6-sol"
        );
        assert_eq!(
            api_model_for_tier("dystil_ai", "ignored", AiModelTier::Economy),
            "gpt-5.6-luna"
        );
        assert_eq!(
            api_model_for_tier("openai_compatible", "my-model", AiModelTier::Frontier),
            "my-model"
        );
    }

    #[test]
    fn pi_errors_are_normalized_without_provider_branches_in_callers() {
        assert_eq!(
            normalize_pi_error("request timed out".into()).code,
            AiRuntimeErrorCode::Timeout
        );
        assert_eq!(
            normalize_pi_error("invalid structured answer".into()).code,
            AiRuntimeErrorCode::InvalidOutput
        );
        assert_eq!(
            normalize_pi_error("provider rejected request: 401".into()).code,
            AiRuntimeErrorCode::Authentication
        );
    }
}
