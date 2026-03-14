use anyhow::Result;
use crate::todo::TodoItem;

/// A single message in the AI chat conversation.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub is_user: bool,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self { Self { is_user: true,  content: content.into() } }
    pub fn ai(content: impl Into<String>)   -> Self { Self { is_user: false, content: content.into() } }
}

/// Chat with the AI about generated UpCloud Terraform files.
/// `tf_context` is the concatenated content of the output .tf files (may be truncated).
/// `messages` is the full conversation history so far.
pub async fn chat_with_tf(
    messages: &[ChatMessage],
    tf_context: &str,
    api_key: &str,
) -> Result<String> {
    let api_url = std::env::var("LLM_API_URL")
        .unwrap_or_else(|_| "https://llm-proxy.edgez.live".to_string());
    let model = std::env::var("LLM_MODEL")
        .unwrap_or_else(|_| "claude-latest".to_string());
    let endpoint = format!("{}/v1/chat/completions", api_url.trim_end_matches('/'));

    let system = format!(
        "You are an expert UpCloud Terraform advisor helping a team migrate from AWS to UpCloud.\n\
        You have full access to the generated UpCloud Terraform below.\n\
        Be concise. Identify real issues. When the user asks to validate, check for:\n\
        - Missing required attributes\n- Unresolved <TODO> placeholders\n\
        - Incorrect resource references\n- UpCloud-specific constraints\n\n\
        --- GENERATED TERRAFORM ---\n{tf}",
        tf = tf_context,
    );

    let mut api_msgs: Vec<serde_json::Value> = vec![
        serde_json::json!({"role": "system", "content": system}),
    ];
    for msg in messages {
        api_msgs.push(serde_json::json!({
            "role": if msg.is_user { "user" } else { "assistant" },
            "content": msg.content,
        }));
    }

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
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

/// Call an OpenAI-compatible LLM API to suggest a replacement value for a TODO placeholder.
///
/// Configuration via environment variables:
///   LLM_API_URL  — base URL, e.g. https://llm-proxy.edgez.live  (default: https://llm-proxy.edgez.live)
///   LLM_API_KEY  — API key
///   LLM_MODEL    — model name (default: claude-latest)
pub async fn get_todo_suggestion(item: &TodoItem, api_key: &str) -> Result<String> {
    let api_url = std::env::var("LLM_API_URL")
        .unwrap_or_else(|_| "https://llm-proxy.edgez.live".to_string());
    let model = std::env::var("LLM_MODEL")
        .unwrap_or_else(|_| "claude-latest".to_string());

    let endpoint = format!("{}/v1/chat/completions", api_url.trim_end_matches('/'));

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

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 128,
        "messages": [{"role": "user", "content": prompt}]
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
    let suggestion = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("(no suggestion)")
        .trim()
        .to_string();

    Ok(suggestion)
}
