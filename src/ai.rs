use anyhow::Result;
use crate::todo::TodoItem;

/// Shared helper to call an OpenAI-compatible LLM API endpoint.
async fn call_llm_api(
    api_msgs: Vec<serde_json::Value>,
    model: String,
    max_tokens: usize,
    api_key: &str,
    api_url: Option<String>,
) -> Result<String> {
    let api_url = api_url.unwrap_or_else(|| std::env::var("LLM_API_URL").unwrap_or_else(|_| "https://llm-proxy.edgez.live".to_string()));
    let model = if model.is_empty() {
        std::env::var("LLM_MODEL").unwrap_or_else(|_| "claude-latest".to_string())
    } else {
        model
    };
    let endpoint = format!("{}/v1/chat/completions", api_url.trim_end_matches('/'));

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": api_msgs,
    });

    let resp = client
        .post(&endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("API {} — {}", status, text));
    }

    let json: serde_json::Value = resp.json().await?;
    Ok(json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("(no response)")
        .trim()
        .to_string())
}

/// A single message in the AI chat conversation.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub is_user: bool,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self { is_user: true, content: content.into() }
    }
    pub fn ai(content: impl Into<String>) -> Self {
        Self { is_user: false, content: content.into() }
    }
}

/// Chat with the AI about generated Terraform files.
pub async fn chat_with_tf(
    messages: &[ChatMessage],
    tf_context: &str,
    api_key: &str,
) -> Result<String> {
    let system = format!(
        "You are an expert UpCloud Terraform advisor helping a team migrate from AWS to UpCloud.\n\
        You have full access to the generated UpCloud Terraform below.\n\
        Be concise. Identify real issues. If there are TODOs, suggest appropriate values.\n\n\
        --- GENERATED TERRAFORM ---\n{tf}",
        tf = tf_context,
    );

    let mut api_msgs: Vec<serde_json::Value> =
        vec![serde_json::json!({"role": "system", "content": system})];
    for msg in messages {
        api_msgs.push(serde_json::json!({
            "role": if msg.is_user { "user" } else { "assistant" },
            "content": msg.content,
        }));
    }

    call_llm_api(api_msgs, String::new(), 1024, api_key, None).await
}

/// Suggest a replacement value for a TODO placeholder.
pub async fn get_todo_suggestion(item: &TodoItem, api_key: &str) -> Result<String> {
    let context_str = item.context.join("\n");
    let prompt = format!(
        "You are helping migrate AWS Terraform to UpCloud Terraform.\n\
        File: {file}\n\
        Line {line}: {line_content}\n\
        Placeholder: {placeholder}\n\
        \nContext:\n{context}\n\
        \nSuggest a concrete value to replace {placeholder}.\n\
        Rules:\n\
        - Respond with ONLY the replacement value, no explanation\n\
        - For base64 cert/key TODOs: respond with `# replace with base64-encoded PEM`\n\
        - For IP addresses: suggest an appropriate private IP like 10.0.0.1\n\
        - For gateway IPs: suggest 10.0.0.1\n\
        - For CIDR blocks: suggest 0.0.0.0/0 for default routes\n\
        - For access restrictions: suggest 0.0.0.0/0",
        file = item.file,
        line = item.line_no,
        line_content = item.line_content.trim(),
        placeholder = item.placeholder,
        context = context_str,
    );

    let api_msgs = vec![serde_json::json!({"role": "user", "content": prompt})];
    call_llm_api(api_msgs, String::new(), 128, api_key, None).await
}
