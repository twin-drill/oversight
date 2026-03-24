use crate::config::LlmConfig;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    #[default]
    Anthropic,
    OpenAI,
    Gemini,
}

impl fmt::Display for LlmProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmProvider::Anthropic => write!(f, "Anthropic"),
            LlmProvider::OpenAI => write!(f, "OpenAI"),
            LlmProvider::Gemini => write!(f, "Gemini"),
        }
    }
}

impl LlmProvider {
    pub fn env_var(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "ANTHROPIC_API_KEY",
            LlmProvider::OpenAI => "OPENAI_API_KEY",
            LlmProvider::Gemini => "GEMINI_API_KEY",
        }
    }

    pub fn default_model(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "claude-sonnet-4-latest",
            LlmProvider::OpenAI => "gpt-4o-mini",
            LlmProvider::Gemini => "gemini-2.0-flash",
        }
    }

    pub fn detect_from_env() -> Option<LlmProvider> {
        for provider in &[LlmProvider::Anthropic, LlmProvider::OpenAI, LlmProvider::Gemini] {
            if std::env::var(provider.env_var()).ok().filter(|k| !k.is_empty()).is_some() {
                return Some(*provider);
            }
        }
        None
    }
}

pub struct LlmClient {
    provider: LlmProvider,
    api_key: String,
    config: LlmConfig,
    http: reqwest::Client,
}

impl std::fmt::Debug for LlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmClient")
            .field("provider", &self.provider)
            .field("model", &self.config.model)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl LlmClient {
    pub fn from_config(config: LlmConfig) -> Result<Self> {
        let api_key = std::env::var(config.provider.env_var())
            .map_err(|_| Error::LlmKeyMissing(config.provider.env_var().to_string()))?;

        if api_key.is_empty() {
            return Err(Error::LlmKeyMissing(config.provider.env_var().to_string()));
        }

        Ok(LlmClient {
            provider: config.provider,
            api_key,
            config,
            http: Self::build_http_client(),
        })
    }

    pub fn with_key(api_key: String, config: LlmConfig) -> Self {
        LlmClient {
            provider: config.provider,
            api_key,
            config,
            http: Self::build_http_client(),
        }
    }

    fn build_http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    }

    pub async fn complete(
        &self,
        system: Option<&str>,
        user_prompt: &str,
    ) -> Result<String> {
        match self.provider {
            LlmProvider::Anthropic => self.complete_anthropic(system, user_prompt).await,
            LlmProvider::OpenAI => self.complete_openai(system, user_prompt).await,
            LlmProvider::Gemini => self.complete_gemini(system, user_prompt).await,
        }
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    async fn complete_anthropic(
        &self,
        system: Option<&str>,
        user_prompt: &str,
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Req {
            model: String,
            max_tokens: u32,
            messages: Vec<Msg>,
            #[serde(skip_serializing_if = "Option::is_none")]
            system: Option<String>,
        }
        #[derive(Serialize, Deserialize)]
        struct Msg { role: String, content: String }
        #[derive(Deserialize)]
        struct Resp { content: Vec<Block> }
        #[derive(Deserialize)]
        struct Block { #[serde(rename = "type")] block_type: String, #[serde(default)] text: Option<String> }

        let resp = self.http
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&Req {
                model: self.config.model.clone(),
                max_tokens: self.config.max_tokens,
                messages: vec![Msg { role: "user".into(), content: user_prompt.into() }],
                system: system.map(|s| s.into()),
            })
            .send().await
            .map_err(|e| Error::LlmApi(format!("Anthropic request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::LlmApi(format!("Anthropic returned {status}: {body}")));
        }

        let parsed: Resp = resp.json().await
            .map_err(|e| Error::LlmApi(format!("Failed to parse Anthropic response: {e}")))?;

        let text: String = parsed.content.iter()
            .filter(|b| b.block_type == "text")
            .filter_map(|b| b.text.as_deref())
            .collect::<Vec<_>>()
            .join("");

        if text.is_empty() {
            return Err(Error::LlmApi("Anthropic returned no text content".into()));
        }
        Ok(text)
    }

    async fn complete_openai(
        &self,
        system: Option<&str>,
        user_prompt: &str,
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Req { model: String, max_tokens: u32, messages: Vec<Msg> }
        #[derive(Serialize)]
        struct Msg { role: String, content: String }
        #[derive(Deserialize)]
        struct Resp { choices: Vec<Choice> }
        #[derive(Deserialize)]
        struct Choice { message: ChoiceMsg }
        #[derive(Deserialize)]
        struct ChoiceMsg { content: Option<String> }

        let mut messages = Vec::new();
        if let Some(sys) = system {
            messages.push(Msg { role: "system".into(), content: sys.into() });
        }
        messages.push(Msg { role: "user".into(), content: user_prompt.into() });

        let resp = self.http
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&Req {
                model: self.config.model.clone(),
                max_tokens: self.config.max_tokens,
                messages,
            })
            .send().await
            .map_err(|e| Error::LlmApi(format!("OpenAI request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::LlmApi(format!("OpenAI returned {status}: {body}")));
        }

        let parsed: Resp = resp.json().await
            .map_err(|e| Error::LlmApi(format!("Failed to parse OpenAI response: {e}")))?;

        parsed.choices.first()
            .and_then(|c| c.message.content.clone())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| Error::LlmApi("OpenAI returned no content".into()))
    }

    async fn complete_gemini(
        &self,
        system: Option<&str>,
        user_prompt: &str,
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Req {
            #[serde(skip_serializing_if = "Option::is_none")]
            system_instruction: Option<SysPart>,
            contents: Vec<Content>,
            #[serde(rename = "generationConfig")]
            generation_config: GenConfig,
        }
        #[derive(Serialize)]
        struct SysPart { parts: Vec<Part> }
        #[derive(Serialize)]
        struct Content { role: String, parts: Vec<Part> }
        #[derive(Serialize)]
        struct Part { text: String }
        #[derive(Serialize)]
        struct GenConfig { #[serde(rename = "maxOutputTokens")] max_output_tokens: u32 }
        #[derive(Deserialize)]
        struct Resp { candidates: Option<Vec<Candidate>> }
        #[derive(Deserialize)]
        struct Candidate { content: Option<CandidateContent> }
        #[derive(Deserialize)]
        struct CandidateContent { parts: Option<Vec<CandidatePart>> }
        #[derive(Deserialize)]
        struct CandidatePart { text: Option<String> }

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.config.model
        );

        let resp = self.http
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .header("content-type", "application/json")
            .json(&Req {
                system_instruction: system.map(|s| SysPart {
                    parts: vec![Part { text: s.into() }],
                }),
                contents: vec![Content {
                    role: "user".into(),
                    parts: vec![Part { text: user_prompt.into() }],
                }],
                generation_config: GenConfig { max_output_tokens: self.config.max_tokens },
            })
            .send().await
            .map_err(|e| Error::LlmApi(format!("Gemini request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::LlmApi(format!("Gemini returned {status}: {body}")));
        }

        let parsed: Resp = resp.json().await
            .map_err(|e| Error::LlmApi(format!("Failed to parse Gemini response: {e}")))?;

        let text: String = parsed.candidates
            .and_then(|c| c.into_iter().next())
            .and_then(|c| c.content)
            .and_then(|c| c.parts)
            .map(|parts| {
                parts.into_iter()
                    .filter_map(|p| p.text)
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();

        if text.is_empty() {
            return Err(Error::LlmApi("Gemini returned no content".into()));
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_var_names() {
        assert_eq!(LlmProvider::Anthropic.env_var(), "ANTHROPIC_API_KEY");
        assert_eq!(LlmProvider::OpenAI.env_var(), "OPENAI_API_KEY");
        assert_eq!(LlmProvider::Gemini.env_var(), "GEMINI_API_KEY");
    }

    #[test]
    fn test_provider_defaults() {
        assert_eq!(LlmProvider::Anthropic.default_model(), "claude-sonnet-4-latest");
        assert_eq!(LlmProvider::OpenAI.default_model(), "gpt-4o-mini");
        assert_eq!(LlmProvider::Gemini.default_model(), "gemini-2.0-flash");
    }

    #[test]
    fn test_with_key() {
        let config = LlmConfig {
            provider: LlmProvider::OpenAI,
            model: "gpt-4o".into(),
            max_tokens: 1024,
        };
        let client = LlmClient::with_key("test-key".into(), config);
        assert_eq!(client.model(), "gpt-4o");
    }
}
