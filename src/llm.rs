use async_openai::{
    Client,
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
    },
};
use std::time::Duration;

/// Python 侧不传 `system_prompt` 时使用的默认提示词。
pub(crate) const DEFAULT_SYS_PROMPT: &str = "你是一个 RSS 订阅源的摘要生成器。请根据用户提供的内容生成简明扼要的摘要。以markdown格式输出，确保内容清晰易读。";

/// 调一次 chat completion，返回摘要正文。
///
/// 原来的 CLI 用 `expect` 直接 panic，任何一次网络抖动都会带崩进程；
/// 这里把错误原样返回给 Python，由上层记日志并决定退出码。
pub(crate) async fn summarize(
    text: &str,
    api_base: &str,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    timeout: Duration,
) -> Result<String, String> {
    let client = Client::with_config(
        OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base(api_base),
    );

    let request = CreateChatCompletionRequestArgs::default()
        .model(model)
        .messages(vec![
            ChatCompletionRequestMessage::System(
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(system_prompt)
                    .build()
                    .map_err(|e| format!("构建系统消息失败: {e}"))?,
            ),
            ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessageArgs::default()
                    .content(text)
                    .build()
                    .map_err(|e| format!("构建用户消息失败: {e}"))?,
            ),
        ])
        .build()
        .map_err(|e| format!("构建请求失败: {e}"))?;

    let response = tokio::time::timeout(timeout, client.chat().create(request))
        .await
        .map_err(|_| format!("LLM 请求超时（>{timeout:?}）"))?
        .map_err(|e| format!("LLM 请求失败: {e}"))?;

    Ok(response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_deref())
        .unwrap_or_default()
        .trim()
        .to_owned())
}
