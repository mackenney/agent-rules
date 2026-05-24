//! Pi-based agentic evaluator
//!
//! Spawns the `pi` CLI as a subprocess for agentic rule evaluation.
//! Falls back to direct Anthropic API call for verdict normalization.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use tokio::process::Command;
use tokio::time::timeout;

use crate::evaluator::{AgenticEvalOpts, AgenticEvaluator};
use crate::llm::LlmError;
use crate::prompt::build_agentic_task;
use crate::schema::{ContextHint, Rule, RuleVerdict, Verdict};

/// Maximum retries for verdict normalization
#[allow(dead_code)]
const MAX_NORMALIZE_RETRIES: u32 = 2;

/// Agentic evaluator using pi subprocess
pub struct PiAgenticEvaluator {
    api_key: String,
    pi_binary: PathBuf,
}

impl PiAgenticEvaluator {
    /// Create a new evaluator
    ///
    /// # Arguments
    /// * `api_key` - Anthropic API key (passed to pi via environment)
    ///
    /// # Errors
    /// Returns error if `pi` binary is not found in PATH
    #[allow(dead_code)]
    pub fn new(api_key: String) -> Result<Self, LlmError> {
        let pi_binary = which::which("pi")
            .map_err(|_| LlmError::Request("pi binary not found in PATH".to_string()))?;

        Ok(Self { api_key, pi_binary })
    }

    /// Build the tools list based on allow_bash setting
    fn build_tools_list(allow_bash: bool) -> String {
        if allow_bash {
            "read,grep,find,ls,bash".to_string()
        } else {
            "read,grep,find,ls".to_string()
        }
    }

    /// Run pi subprocess and collect output
    async fn run_pi_session(
        &self,
        task: &str,
        repo_root: &Path,
        opts: &AgenticEvalOpts,
    ) -> Result<String, LlmError> {
        let tools = Self::build_tools_list(opts.allow_bash);

        let mut cmd = Command::new(&self.pi_binary);
        cmd.arg("-p")
            .arg("--no-session")
            .arg("--model")
            .arg(&opts.model)
            .arg("--tools")
            .arg(&tools)
            .arg("--mode")
            .arg("json")
            .arg(task)
            .current_dir(repo_root)
            .env("ANTHROPIC_API_KEY", &self.api_key)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if opts.trace {
            eprintln!("[TRACE] pi command: {:?}", cmd);
        }

        let result = timeout(opts.timeout, async {
            let child = cmd
                .spawn()
                .map_err(|e| LlmError::Request(format!("failed to spawn pi: {}", e)))?;

            let output = child
                .wait_with_output()
                .await
                .map_err(|e| LlmError::Request(format!("pi subprocess failed: {}", e)))?;

            Ok::<_, LlmError>(output)
        })
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if opts.trace {
                    eprintln!("[TRACE] pi stdout: {}", stdout);
                    eprintln!("[TRACE] pi stderr: {}", stderr);
                }

                if !output.status.success() && stdout.is_empty() {
                    return Err(LlmError::Request(format!(
                        "pi exited with status {}: {}",
                        output.status, stderr
                    )));
                }

                Ok(stdout)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(LlmError::Timeout),
        }
    }

    /// Parse verdict from pi output JSON
    fn parse_verdict_from_output(&self, output: &str, rule: &Rule) -> Result<RuleVerdict, String> {
        let json: serde_json::Value =
            serde_json::from_str(output).map_err(|e| format!("JSON parse error: {}", e))?;

        let verdict_str = json
            .get("verdict")
            .and_then(|v| v.as_str())
            .or_else(|| {
                json.get("result")
                    .and_then(|r| r.get("verdict"))
                    .and_then(|v| v.as_str())
            })
            .ok_or("no verdict field found")?;

        let verdict = match verdict_str {
            "pass" => Verdict::Pass,
            "fail" => Verdict::Fail,
            "needs-more-context" => Verdict::Fail,
            _ => return Err(format!("unrecognized verdict: {}", verdict_str)),
        };

        let confidence = json
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);

        let reasoning = json
            .get("reasoning")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .replace('\n', " ")
            .trim()
            .to_string();

        let line_refs: Vec<u32> = json
            .get("line_refs")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u32))
                    .collect()
            })
            .unwrap_or_default();

        Ok(RuleVerdict {
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            verdict,
            confidence,
            reasoning,
            severity: rule.severity,
            line_refs: line_refs.clone(),
            line: line_refs.first().copied(),
            cached: false,
            from_agentic: true,
            context_hint: None,
        })
    }

    /// Normalize verdict via direct Anthropic API call
    async fn normalize_verdict(
        &self,
        raw_output: &str,
        rule: &Rule,
        file_path: &str,
        opts: &AgenticEvalOpts,
    ) -> Result<RuleVerdict, LlmError> {
        let client = reqwest::Client::new();
        let url = "https://api.anthropic.com/v1/messages";

        let system = "Extract a structured rule evaluation verdict from the agent's analysis. \
                      The agent has already done the investigation.";

        let user_content = format!(
            "File: {}\nRule: {} \u{2014} {}\n\nAgent analysis:\n{}\n\nExtract the verdict.",
            file_path,
            rule.id,
            rule.name,
            &raw_output[..raw_output.len().min(8000)]
        );

        let tool = serde_json::json!({
            "name": "submit_verdict",
            "description": "Submit the extracted verdict",
            "input_schema": {
                "type": "object",
                "properties": {
                    "verdict": {
                        "type": "string",
                        "enum": ["pass", "fail"]
                    },
                    "confidence": {
                        "type": "number",
                        "minimum": 0,
                        "maximum": 1
                    },
                    "reasoning": {
                        "type": "string"
                    },
                    "line_refs": {
                        "type": "array",
                        "items": {"type": "integer"}
                    }
                },
                "required": ["verdict", "confidence", "reasoning", "line_refs"]
            }
        });

        let body = serde_json::json!({
            "model": opts.model,
            "max_tokens": 1024,
            "system": system,
            "messages": [{"role": "user", "content": user_content}],
            "tools": [tool],
            "tool_choice": {"type": "tool", "name": "submit_verdict"}
        });

        let response = client
            .post(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Request(e.to_string()))?;

        if !response.status().is_success() {
            return Err(LlmError::Request(format!(
                "normalization request failed: {}",
                response.status()
            )));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| LlmError::Parse(e.to_string()))?;

        if let Some(content) = json.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    if let Some(input) = block.get("input") {
                        let verdict_str = input
                            .get("verdict")
                            .and_then(|v| v.as_str())
                            .unwrap_or("fail");

                        let verdict = match verdict_str {
                            "pass" => Verdict::Pass,
                            _ => Verdict::Fail,
                        };

                        let confidence = input
                            .get("confidence")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.5);

                        let reasoning = input
                            .get("reasoning")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        let line_refs: Vec<u32> = input
                            .get("line_refs")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_u64().map(|n| n as u32))
                                    .collect()
                            })
                            .unwrap_or_default();

                        return Ok(RuleVerdict {
                            rule_id: rule.id.clone(),
                            rule_name: rule.name.clone(),
                            verdict,
                            confidence,
                            reasoning,
                            severity: rule.severity,
                            line_refs: line_refs.clone(),
                            line: line_refs.first().copied(),
                            cached: false,
                            from_agentic: true,
                            context_hint: None,
                        });
                    }
                }
            }
        }

        Err(LlmError::Parse(
            "no tool use in normalization response".to_string(),
        ))
    }

    /// Create a fallback verdict for errors/timeouts
    fn fallback_verdict(&self, rule: &Rule, reason: &str) -> RuleVerdict {
        RuleVerdict {
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            verdict: Verdict::Fail,
            confidence: 0.0,
            reasoning: reason.to_string(),
            severity: rule.severity,
            line_refs: vec![],
            line: None,
            cached: false,
            from_agentic: true,
            context_hint: None,
        }
    }
}

#[async_trait]
impl AgenticEvaluator for PiAgenticEvaluator {
    async fn evaluate(
        &self,
        file_path: &str,
        diff: &str,
        content: Option<&str>,
        rule: &Rule,
        hints: &[ContextHint],
        repo_root: &Path,
        opts: &AgenticEvalOpts,
    ) -> Result<RuleVerdict, LlmError> {
        let task = build_agentic_task(file_path, diff, content, rule, hints);

        if opts.trace {
            eprintln!("[TRACE] Agentic task for {}: {}", file_path, task);
        }

        let output = match self.run_pi_session(&task, repo_root, opts).await {
            Ok(out) => out,
            Err(LlmError::Timeout) => {
                eprintln!(
                    "Agentic escalation timed out after {}ms for {}",
                    opts.timeout.as_millis(),
                    file_path
                );
                return Ok(self.fallback_verdict(rule, "Agentic session timed out"));
            }
            Err(e) => {
                eprintln!("Agentic escalation error for {}: {}", file_path, e);
                return Ok(self.fallback_verdict(rule, &format!("Agentic error: {}", e)));
            }
        };

        match self.parse_verdict_from_output(&output, rule) {
            Ok(verdict) => return Ok(verdict),
            Err(parse_err) => {
                if opts.trace {
                    eprintln!(
                        "[TRACE] Parse failed: {}, attempting normalization",
                        parse_err
                    );
                }
            }
        }

        match self.normalize_verdict(&output, rule, file_path, opts).await {
            Ok(verdict) => Ok(verdict),
            Err(e) => {
                eprintln!("Normalization failed for {}: {}", file_path, e);
                Ok(self.fallback_verdict(rule, "Verdict normalization failed"))
            }
        }
    }
}
