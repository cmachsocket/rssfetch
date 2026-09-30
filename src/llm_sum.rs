use crate::get_cfg::RSSconfig;
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequestArgs, ChatCompletionRequestSystemMessageArgs
    },
};
use tokio::{fs::OpenOptions, io::AsyncWriteExt};

const SYS_PROMPT: &str = "你是一个 RSS 订阅源的摘要生成器。请根据用户提供的内容生成简明扼要的摘要。以markdown格式输出，确保内容清晰易读。";

pub(crate) async fn llm_sum(rssconfig: &RSSconfig) {
    // 1. 创建自定义配置
    let config = OpenAIConfig::new()
        .with_api_key(&rssconfig.api_key) // 使用配置中的 API Key
        .with_api_base(&rssconfig.api_base); // 使用配置中的 API URL

    // 2. 用配置创建客户端
    let client = Client::with_config(config);
    let content = tokio::fs::read_to_string(&rssconfig.file_path)
        .await
        .expect("无法读取文件");
    let request = CreateChatCompletionRequestArgs::default()
        .model(&rssconfig.model_name)
        .messages(vec![
            ChatCompletionRequestMessage::System(
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(SYS_PROMPT)
                    .build()
                    .expect("无法构建系统消息"),
            ),
            ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessageArgs::default()
                    .content(content)
                    .build()
                    .expect("无法构建用户消息"),
            ),
        ])
        .build()
        .expect("无法构建请求");
    let response = client.chat().create(request).await.expect("请求失败");
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&rssconfig.summary_path)
        .await
        .expect("无法打开输出文件");
    let summary = response.choices[0].message.content.as_deref().unwrap_or("");
    f.write_all(summary.as_bytes()).await.expect("无法写入文件");
}
