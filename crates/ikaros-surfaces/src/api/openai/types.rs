// SPDX-License-Identifier: GPL-3.0-only

use super::super::*;

pub(in crate::api) fn api_request_options(
    model_config: &ModelConfig,
    request: &ApiChatCompletionRequest,
) -> Result<ModelRequestOptions> {
    let mut options = model_request_options_from_config(model_config)?;
    if request.max_tokens.is_some() {
        options.max_tokens = request.max_tokens;
    }
    if request.temperature.is_some() {
        options.temperature = request.temperature;
    }
    if request.top_p.is_some() {
        options.top_p = request.top_p;
    }
    if request.n.is_some() {
        options.n = request.n;
    }
    if request.presence_penalty.is_some() {
        options.presence_penalty = request.presence_penalty;
    }
    if request.frequency_penalty.is_some() {
        options.frequency_penalty = request.frequency_penalty;
    }
    if request.seed.is_some() {
        options.seed = request.seed;
    }
    if let Some(stop) = &request.stop {
        options.stop = stop.values();
    }
    if let Some(tool_choice) = &request.tool_choice {
        options
            .extra_body
            .insert("tool_choice".into(), tool_choice.clone());
    }
    Ok(options)
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiChatCompletionRequest {
    pub(in crate::api) model: Option<String>,
    pub(in crate::api) messages: Vec<ApiChatMessage>,
    #[serde(default)]
    pub(in crate::api) tools: Vec<ApiToolDefinition>,
    #[serde(default)]
    pub(in crate::api) tool_choice: Option<Value>,
    pub(in crate::api) stream: Option<bool>,
    pub(in crate::api) max_tokens: Option<u32>,
    pub(in crate::api) temperature: Option<f32>,
    pub(in crate::api) top_p: Option<f32>,
    pub(in crate::api) n: Option<u32>,
    pub(in crate::api) presence_penalty: Option<f32>,
    pub(in crate::api) frequency_penalty: Option<f32>,
    pub(in crate::api) seed: Option<u64>,
    pub(in crate::api) stop: Option<ApiStop>,
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiResponseCreateRequest {
    pub(in crate::api) model: Option<String>,
    pub(in crate::api) input: ApiResponsesInput,
    #[serde(default)]
    pub(in crate::api) instructions: Option<String>,
    #[serde(default)]
    pub(in crate::api) tools: Vec<ApiResponseToolDefinition>,
    pub(in crate::api) stream: Option<bool>,
    pub(in crate::api) max_output_tokens: Option<u32>,
    pub(in crate::api) temperature: Option<f32>,
    pub(in crate::api) top_p: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(in crate::api) enum ApiResponsesInput {
    Text(String),
    Items(Vec<ApiResponseInputItem>),
    Other(Value),
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiResponseInputItem {
    #[serde(default)]
    pub(in crate::api) role: Option<String>,
    #[serde(default)]
    pub(in crate::api) content: Option<ApiResponseInputContent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(in crate::api) enum ApiResponseInputContent {
    Text(String),
    Parts(Vec<ApiResponseContentPart>),
    Other(Value),
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiResponseContentPart {
    #[serde(rename = "type")]
    pub(in crate::api) kind: Option<String>,
    #[serde(default)]
    pub(in crate::api) text: Option<String>,
    #[serde(default)]
    pub(in crate::api) image_url: Option<String>,
    #[serde(default)]
    pub(in crate::api) audio_url: Option<String>,
    #[serde(default)]
    pub(in crate::api) input_audio: Option<ApiInputAudio>,
    #[serde(default)]
    pub(in crate::api) file_url: Option<String>,
    #[serde(default)]
    pub(in crate::api) file_id: Option<String>,
    #[serde(default)]
    pub(in crate::api) file_data: Option<String>,
    #[serde(default)]
    pub(in crate::api) filename: Option<String>,
    #[serde(default)]
    pub(in crate::api) detail: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiResponseToolDefinition {
    #[serde(rename = "type")]
    pub(in crate::api) kind: Option<String>,
    #[serde(default)]
    pub(in crate::api) name: Option<String>,
    #[serde(default)]
    pub(in crate::api) description: Option<String>,
    #[serde(default)]
    pub(in crate::api) parameters: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiEmbeddingRequest {
    pub(in crate::api) model: Option<String>,
    pub(in crate::api) input: ApiEmbeddingInput,
    #[serde(default)]
    pub(in crate::api) encoding_format: Option<String>,
    #[serde(default, rename = "user")]
    pub(in crate::api) _user: Option<String>,
}

impl ApiEmbeddingRequest {
    pub(in crate::api) fn inputs(&self) -> Result<Vec<String>> {
        let inputs = self.input.values()?;
        if inputs.iter().any(|input| input.trim().is_empty()) {
            anyhow::bail!("embedding input entries must not be empty");
        }
        Ok(inputs)
    }

    pub(in crate::api) fn embedding_encoding(&self) -> Result<ApiEmbeddingEncoding> {
        ApiEmbeddingEncoding::parse(self.encoding_format.as_deref())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::api) enum ApiEmbeddingEncoding {
    Float,
    Base64,
}

impl ApiEmbeddingEncoding {
    pub(in crate::api) fn parse(value: Option<&str>) -> Result<Self> {
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Self::Float);
        };
        if value.eq_ignore_ascii_case("float") {
            return Ok(Self::Float);
        }
        if value.eq_ignore_ascii_case("base64") {
            return Ok(Self::Base64);
        }
        anyhow::bail!("embedding encoding_format must be 'float' or 'base64'")
    }

    pub(in crate::api) fn as_str(self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Base64 => "base64",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(in crate::api) enum ApiEmbeddingInput {
    One(String),
    Many(Vec<String>),
    Other(Value),
}

impl ApiEmbeddingInput {
    pub(in crate::api) fn values(&self) -> Result<Vec<String>> {
        match self {
            Self::One(value) => Ok(vec![value.clone()]),
            Self::Many(values) => Ok(values.clone()),
            Self::Other(value) => {
                let _ = value;
                anyhow::bail!(
                    "embedding input must be a string or an array of strings in this first API slice"
                )
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiChatMessage {
    pub(in crate::api) role: String,
    #[serde(default)]
    pub(in crate::api) content: Option<ApiMessageContent>,
    #[serde(default)]
    pub(in crate::api) name: Option<String>,
    #[serde(default)]
    pub(in crate::api) tool_call_id: Option<String>,
    #[serde(default)]
    pub(in crate::api) tool_calls: Vec<ApiToolCall>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(in crate::api) enum ApiMessageContent {
    Text(String),
    Parts(Vec<ApiContentPart>),
    Other(Value),
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiContentPart {
    #[serde(rename = "type")]
    pub(in crate::api) kind: Option<String>,
    #[serde(default)]
    pub(in crate::api) text: Option<String>,
    #[serde(default)]
    pub(in crate::api) image_url: Option<ApiImageUrl>,
    #[serde(default)]
    pub(in crate::api) audio_url: Option<ApiAudioUrl>,
    #[serde(default)]
    pub(in crate::api) input_audio: Option<ApiInputAudio>,
    #[serde(default)]
    pub(in crate::api) file_url: Option<ApiFileUrl>,
    #[serde(default)]
    pub(in crate::api) file: Option<ApiInputFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(in crate::api) enum ApiImageUrl {
    Text(String),
    Object {
        url: String,
        #[serde(default)]
        detail: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(in crate::api) enum ApiAudioUrl {
    Text(String),
    Object {
        url: String,
        #[serde(default)]
        mime_type: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiInputAudio {
    #[serde(default)]
    pub(in crate::api) data: Option<String>,
    #[serde(default)]
    pub(in crate::api) format: Option<String>,
    #[serde(default)]
    pub(in crate::api) url: Option<String>,
    #[serde(default)]
    pub(in crate::api) audio_url: Option<String>,
    #[serde(default)]
    pub(in crate::api) mime_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(in crate::api) enum ApiFileUrl {
    Text(String),
    Object {
        url: String,
        #[serde(default)]
        mime_type: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        filename: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiInputFile {
    #[serde(default)]
    pub(in crate::api) file_url: Option<String>,
    #[serde(default)]
    pub(in crate::api) url: Option<String>,
    #[serde(default)]
    pub(in crate::api) file_id: Option<String>,
    #[serde(default)]
    pub(in crate::api) file_data: Option<String>,
    #[serde(default)]
    pub(in crate::api) filename: Option<String>,
    #[serde(default)]
    pub(in crate::api) name: Option<String>,
    #[serde(default)]
    pub(in crate::api) mime_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiToolDefinition {
    #[serde(rename = "type")]
    pub(in crate::api) kind: Option<String>,
    pub(in crate::api) function: ApiToolFunctionDefinition,
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiToolFunctionDefinition {
    pub(in crate::api) name: String,
    #[serde(default)]
    pub(in crate::api) description: Option<String>,
    #[serde(default)]
    pub(in crate::api) parameters: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiToolCall {
    #[serde(default)]
    pub(in crate::api) id: Option<String>,
    #[serde(rename = "type", default)]
    pub(in crate::api) kind: Option<String>,
    pub(in crate::api) function: ApiToolCallFunction,
}

#[derive(Debug, Clone, Deserialize)]
pub(in crate::api) struct ApiToolCallFunction {
    pub(in crate::api) name: String,
    #[serde(default)]
    pub(in crate::api) arguments: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(in crate::api) enum ApiStop {
    One(String),
    Many(Vec<String>),
}

impl ApiStop {
    pub(in crate::api) fn values(&self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value.clone()],
            Self::Many(values) => values.clone(),
        }
    }
}

pub(in crate::api) fn api_message_to_model_message(
    message: ApiChatMessage,
) -> Result<ModelMessage> {
    let role = message.role.trim();
    if role.is_empty() {
        anyhow::bail!("message role must not be empty");
    }
    let (content, content_blocks) = match message.content {
        Some(content) => api_message_content_to_model_content(content)?,
        None => (String::new(), Vec::new()),
    };
    Ok(ModelMessage {
        role: role.to_owned(),
        content,
        content_blocks,
        tool_calls: message
            .tool_calls
            .into_iter()
            .map(api_tool_call_to_model_tool_call)
            .collect::<Result<Vec<_>>>()?,
        tool_call_id: message.tool_call_id,
        tool_name: message.name,
    })
}

pub(in crate::api) fn api_responses_input_to_model_messages(
    input: ApiResponsesInput,
) -> Result<Vec<ModelMessage>> {
    match input {
        ApiResponsesInput::Text(text) => Ok(vec![ModelMessage::user(text)]),
        ApiResponsesInput::Items(items) => items
            .into_iter()
            .map(api_response_input_item_to_model_message)
            .collect(),
        ApiResponsesInput::Other(value) => {
            let _ = value;
            anyhow::bail!("responses input must be a string or an array of message items")
        }
    }
}

pub(in crate::api) fn api_response_input_item_to_model_message(
    item: ApiResponseInputItem,
) -> Result<ModelMessage> {
    let role = item.role.unwrap_or_else(|| "user".into());
    let (content, content_blocks) = match item.content {
        Some(content) => api_response_content_to_model_content(content)?,
        None => (String::new(), Vec::new()),
    };
    Ok(ModelMessage {
        role,
        content,
        content_blocks,
        tool_calls: Vec::new(),
        tool_call_id: None,
        tool_name: None,
    })
}

pub(in crate::api) fn api_response_content_to_model_content(
    content: ApiResponseInputContent,
) -> Result<(String, Vec<ModelContentBlock>)> {
    match content {
        ApiResponseInputContent::Text(text) => Ok((text, Vec::new())),
        ApiResponseInputContent::Parts(parts) => {
            let mut text_parts = Vec::new();
            let mut content_blocks = Vec::new();
            for part in parts {
                let kind = part.kind.as_deref().unwrap_or("input_text").to_owned();
                match kind.as_str() {
                    "input_text" | "text" => {
                        let text = part.text.unwrap_or_default();
                        text_parts.push(text.clone());
                        content_blocks.push(ModelContentBlock::text(text));
                    }
                    "input_image" | "image_url" => {
                        let image_url = part.image_url.ok_or_else(|| {
                            anyhow::anyhow!("input_image part requires image_url")
                        })?;
                        content_blocks.push(ModelContentBlock::Image {
                            image_url,
                            mime_type: None,
                            detail: part.detail,
                        });
                    }
                    "input_audio" | "audio_url" => {
                        content_blocks.push(api_response_audio_part_to_content_block(part)?);
                    }
                    "input_file" | "file_url" | "file" => {
                        content_blocks.push(api_response_file_part_to_content_block(part)?);
                    }
                    other => anyhow::bail!("unsupported responses input part type `{other}`"),
                }
            }
            Ok((text_parts.join("\n"), content_blocks))
        }
        ApiResponseInputContent::Other(value) => serde_json::to_string(&value)
            .map(|text| (text, Vec::new()))
            .with_context(|| "failed to serialize non-text responses input content"),
    }
}

pub(in crate::api) fn api_message_content_to_model_content(
    content: ApiMessageContent,
) -> Result<(String, Vec<ModelContentBlock>)> {
    match content {
        ApiMessageContent::Text(text) => Ok((text, Vec::new())),
        ApiMessageContent::Parts(parts) => {
            let mut text_parts = Vec::new();
            let mut content_blocks = Vec::new();
            for part in parts {
                let kind = part.kind.as_deref().unwrap_or("text").to_owned();
                match kind.as_str() {
                    "text" => {
                        let text = part.text.unwrap_or_default();
                        text_parts.push(text.clone());
                        content_blocks.push(ModelContentBlock::text(text));
                    }
                    "image_url" => {
                        let image = part
                            .image_url
                            .ok_or_else(|| anyhow::anyhow!("image_url part requires image_url"))?;
                        let (image_url, detail) = match image {
                            ApiImageUrl::Text(url) => (url, None),
                            ApiImageUrl::Object { url, detail } => (url, detail),
                        };
                        content_blocks.push(ModelContentBlock::Image {
                            image_url,
                            mime_type: None,
                            detail,
                        });
                    }
                    "input_audio" | "audio_url" => {
                        content_blocks.push(api_audio_part_to_content_block(part)?);
                    }
                    "input_file" | "file_url" | "file" => {
                        content_blocks.push(api_file_part_to_content_block(part)?);
                    }
                    other => anyhow::bail!("unsupported message content part type `{other}`"),
                }
            }
            Ok((text_parts.join("\n"), content_blocks))
        }
        ApiMessageContent::Other(value) => serde_json::to_string(&value)
            .map(|text| (text, Vec::new()))
            .with_context(|| "failed to serialize non-text message content"),
    }
}

pub(in crate::api) fn api_audio_part_to_content_block(
    part: ApiContentPart,
) -> Result<ModelContentBlock> {
    if let Some(audio_url) = part.audio_url {
        let (audio_url, mime_type) = match audio_url {
            ApiAudioUrl::Text(url) => (url, None),
            ApiAudioUrl::Object { url, mime_type } => (url, mime_type),
        };
        return Ok(ModelContentBlock::Audio {
            audio_url,
            mime_type,
        });
    }
    let Some(audio) = part.input_audio else {
        anyhow::bail!("input_audio part requires input_audio or audio_url");
    };
    api_input_audio_to_content_block(audio)
}

pub(in crate::api) fn api_response_audio_part_to_content_block(
    part: ApiResponseContentPart,
) -> Result<ModelContentBlock> {
    if let Some(audio_url) = part.audio_url {
        return Ok(ModelContentBlock::Audio {
            audio_url,
            mime_type: None,
        });
    }
    let Some(audio) = part.input_audio else {
        anyhow::bail!("input_audio part requires input_audio or audio_url");
    };
    api_input_audio_to_content_block(audio)
}

pub(in crate::api) fn api_input_audio_to_content_block(
    audio: ApiInputAudio,
) -> Result<ModelContentBlock> {
    let mime_type = audio
        .mime_type
        .or_else(|| audio.format.as_deref().map(api_audio_format_mime_type));
    if let Some(audio_url) = audio.url.or(audio.audio_url) {
        return Ok(ModelContentBlock::Audio {
            audio_url,
            mime_type,
        });
    }
    let Some(data) = audio.data else {
        anyhow::bail!("input_audio requires data or url");
    };
    if data.starts_with("data:") {
        return Ok(ModelContentBlock::Audio {
            audio_url: data,
            mime_type,
        });
    }
    let mime = mime_type.clone().unwrap_or_else(|| "audio/wav".into());
    Ok(ModelContentBlock::Audio {
        audio_url: format!("data:{mime};base64,{data}"),
        mime_type,
    })
}

pub(in crate::api) fn api_file_part_to_content_block(
    part: ApiContentPart,
) -> Result<ModelContentBlock> {
    if let Some(file_url) = part.file_url {
        let (file_url, mime_type, name) = match file_url {
            ApiFileUrl::Text(url) => (url, None, None),
            ApiFileUrl::Object {
                url,
                mime_type,
                name,
                filename,
            } => (url, mime_type, name.or(filename)),
        };
        return Ok(ModelContentBlock::File {
            file_url,
            mime_type,
            name,
        });
    }
    let Some(file) = part.file else {
        anyhow::bail!("input_file part requires file or file_url");
    };
    api_input_file_to_content_block(file)
}

pub(in crate::api) fn api_response_file_part_to_content_block(
    part: ApiResponseContentPart,
) -> Result<ModelContentBlock> {
    if let Some(file_url) = part.file_url {
        return Ok(ModelContentBlock::File {
            file_url,
            mime_type: None,
            name: part.filename,
        });
    }
    let file = ApiInputFile {
        file_url: None,
        url: None,
        file_id: part.file_id,
        file_data: part.file_data,
        filename: part.filename,
        name: None,
        mime_type: None,
    };
    api_input_file_to_content_block(file)
}

pub(in crate::api) fn api_input_file_to_content_block(
    file: ApiInputFile,
) -> Result<ModelContentBlock> {
    if let Some(file_url) = file.file_url.or(file.url) {
        return Ok(ModelContentBlock::File {
            file_url,
            mime_type: file.mime_type,
            name: file.name.or(file.filename),
        });
    }
    if let Some(file_data) = file.file_data {
        if file_data.starts_with("data:") {
            return Ok(ModelContentBlock::File {
                file_url: file_data,
                mime_type: file.mime_type,
                name: file.name.or(file.filename),
            });
        }
        let mime = file
            .mime_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".into());
        return Ok(ModelContentBlock::File {
            file_url: format!("data:{mime};base64,{file_data}"),
            mime_type: file.mime_type,
            name: file.name.or(file.filename),
        });
    }
    if let Some(file_id) = file.file_id {
        return Ok(ModelContentBlock::File {
            file_url: format!("file_id:{file_id}"),
            mime_type: file.mime_type,
            name: file.name.or(file.filename),
        });
    }
    anyhow::bail!("input_file requires file_url, file_data, or file_id")
}

pub(in crate::api) fn api_audio_format_mime_type(format: &str) -> String {
    match format.trim().to_ascii_lowercase().as_str() {
        "mp3" => "audio/mpeg".into(),
        "m4a" | "mp4" => "audio/mp4".into(),
        "ogg" => "audio/ogg".into(),
        "opus" => "audio/opus".into(),
        "flac" => "audio/flac".into(),
        "wav" | "" => "audio/wav".into(),
        other => format!("audio/{other}"),
    }
}

pub(in crate::api) fn api_response_tool_to_model_tool(
    tool: ApiResponseToolDefinition,
) -> Result<ModelToolDefinition> {
    if tool.kind.as_deref().unwrap_or("function") != "function" {
        anyhow::bail!("only function tools are supported by /v1/responses in this first API slice");
    }
    let name = tool.name.unwrap_or_default();
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("response function tool name must not be empty");
    }
    Ok(ModelToolDefinition {
        name: name.to_owned(),
        description: tool.description.unwrap_or_default(),
        input_schema: tool
            .parameters
            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
    })
}

pub(in crate::api) fn api_tool_definition_to_model_tool(
    tool: ApiToolDefinition,
) -> Result<ModelToolDefinition> {
    if tool.kind.as_deref().unwrap_or("function") != "function" {
        anyhow::bail!("only function tools are supported");
    }
    let name = tool.function.name.trim();
    if name.is_empty() {
        anyhow::bail!("tool function name must not be empty");
    }
    Ok(ModelToolDefinition {
        name: name.to_owned(),
        description: tool.function.description.unwrap_or_default(),
        input_schema: tool
            .function
            .parameters
            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
    })
}

pub(in crate::api) fn api_tool_call_to_model_tool_call(call: ApiToolCall) -> Result<ModelToolCall> {
    if call.kind.as_deref().unwrap_or("function") != "function" {
        anyhow::bail!("only function tool calls are supported");
    }
    let name = call.function.name.trim();
    if name.is_empty() {
        anyhow::bail!("tool call function name must not be empty");
    }
    let (input, raw_arguments) = api_tool_call_arguments(call.function.arguments)?;
    Ok(ModelToolCall {
        id: call.id,
        name: name.to_owned(),
        input,
        raw_arguments,
    })
}

pub(in crate::api) fn api_tool_call_arguments(
    arguments: Option<Value>,
) -> Result<(Value, Option<String>)> {
    let Some(arguments) = arguments else {
        return Ok((json!({}), None));
    };
    match arguments {
        Value::String(raw) => {
            if raw.trim().is_empty() {
                return Ok((json!({}), None));
            }
            let input = serde_json::from_str(&raw).unwrap_or_else(|_| Value::String(raw.clone()));
            Ok((input, Some(raw)))
        }
        value @ Value::Object(_) => Ok((value.clone(), Some(value.to_string()))),
        value => Ok((value.clone(), Some(value.to_string()))),
    }
}
