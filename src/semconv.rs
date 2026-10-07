//! Pinned OpenTelemetry semantic-convention keys used by the fixtures.
//!
//! The snapshot is intentionally small: it keeps this CLI standalone while
//! making the schema provenance explicit. Update it when the conventions move.
//! Sources: <https://opentelemetry.io/docs/specs/semconv/> and <https://semconv.com/>.

pub mod attributes {
    pub const SEMCONV_VERSION: &str = "1.44.0";
    pub const SEMCONV_SCHEMA_URL: &str = "https://opentelemetry.io/schemas/1.44.0";
    pub const GENAI_SEMCONV_SCHEMA_URL: &str =
        "https://opentelemetry.io/schemas/gen-ai-dev/1.42.0-dev";
    pub const GENAI_SEMCONV_COMMIT: &str = "e07f4ebacb08f56db8c4c882d117720333fbca04";
    /// Metrics provenance is separate because the upstream metric definitions
    /// changed after the attribute snapshot was pinned.
    pub const GENAI_METRICS_COMMIT: &str = "4f85037ef86e92c510d2ef881a58f1076f6fc0e4";
    pub const DEPLOYMENT_ENVIRONMENT_NAME: &str = "deployment.environment.name";
    pub const ERROR_TYPE: &str = "error.type";
    /// Squasher extension: identifies fixtures as generated traffic.
    pub const SQUASHER_SYNTHETIC: &str = "squasher.synthetic";
    /// Squasher extension: names the generator that produced the fixture.
    pub const SQUASHER_GENERATOR_NAME: &str = "squasher.generator.name";
    /// Squasher extension: records the generator version for fixture provenance.
    pub const SQUASHER_GENERATOR_VERSION: &str = "squasher.generator.version";
    pub const GEN_AI_AGENT_ID: &str = "gen_ai.agent.id";
    pub const GEN_AI_AGENT_NAME: &str = "gen_ai.agent.name";
    pub const GEN_AI_CONVERSATION_ID: &str = "gen_ai.conversation.id";
    pub const GEN_AI_OPERATION_NAME: &str = "gen_ai.operation.name";
    pub const GEN_AI_PROVIDER_NAME: &str = "gen_ai.provider.name";
    pub const GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";
    pub const GEN_AI_RESPONSE_MODEL: &str = "gen_ai.response.model";
    pub const HTTP_REQUEST_METHOD: &str = "http.request.method";
    pub const HTTP_RESPONSE_STATUS_CODE: &str = "http.response.status_code";
    pub const HTTP_ROUTE: &str = "http.route";
    pub const SERVICE_NAME: &str = "service.name";
    pub const SERVICE_VERSION: &str = "service.version";
    pub const DB_OPERATION_NAME: &str = "db.operation.name";
    pub const DB_SYSTEM_NAME: &str = "db.system.name";
    pub const GEN_AI_DATA_SOURCE_ID: &str = "gen_ai.data_source.id";
    pub const GEN_AI_RETRIEVAL_TOP_K: &str = "gen_ai.retrieval.top_k";
    pub const GEN_AI_TOOL_CALL_ID: &str = "gen_ai.tool.call.id";
    pub const GEN_AI_TOOL_NAME: &str = "gen_ai.tool.name";
    pub const GEN_AI_TOOL_TYPE: &str = "gen_ai.tool.type";
    pub const GEN_AI_TOKEN_MODALITY: &str = "gen_ai.token.modality";
    pub const GEN_AI_USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
    pub const GEN_AI_USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";
}
