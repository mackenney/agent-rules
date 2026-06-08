//! Handler for `agent-rules check`

use std::sync::Arc;

use anyhow::{Context, Result, bail};

use agent_rules::config::{CheckConfig, OutputFormat, Provider, get_api_key};
use agent_rules::evaluator::{
    AgenticEvaluator, AnthropicClient, OpenRouterClient, PiAgenticEvaluator, StatelessEvaluator,
};
use agent_rules::git::get_repo_root;
use agent_rules::progress::{NullProgress, create_progress_reporter};
use agent_rules::reporter::{Stylesheet, exit_code_for_report, print_report};
use agent_rules::runner::{CheckInfra, check_pr};

use crate::CheckArgs;

pub async fn run_check(args: CheckArgs, colors: &Stylesheet) -> Result<i32> {
    let provider: Provider = args.provider.into();

    let model = args.model.unwrap_or_else(|| match provider {
        Provider::Anthropic => agent_rules::config::DEFAULT_MODEL.to_string(),
        Provider::OpenRouter => agent_rules::config::DEFAULT_OPENROUTER_MODEL.to_string(),
    });

    let agentic_model = if provider == Provider::OpenRouter && !args.agentic_model.contains('/') {
        format!("anthropic/{}", args.agentic_model)
    } else {
        args.agentic_model.clone()
    };

    let repo_root = match args.repo {
        Some(r) => r,
        None => get_repo_root(&std::env::current_dir()?)?,
    };

    let config = CheckConfig {
        base_ref: args.base,
        head_ref: args.head,
        pr_url: args.pr,
        repo_root,
        files: args.files,
        dir_filters: args
            .dir_filter
            .iter()
            .flat_map(|s| s.split(',').map(|p| p.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        output_format: args.output.into(),
        warn_as_error: args.warn_as_error,
        no_cache: args.no_cache,
        model,
        provider,
        max_concurrent: args.max_concurrent,
        max_agentic_concurrent: args.agentic_concurrency,
        agentic_model,
        agentic_timeout_ms: args.agentic_timeout,
        max_file_bytes: args.max_file_bytes,
        max_diff_chars: args.max_diff_chars,
        max_content_chars: args.max_content_chars,
        timeout_ms: args.timeout,
        verbose: args.verbose || args.trace,
        trace: args.trace,
        post_comment: args.post_comment,
        strict_rules: args.strict_rules,
        allow_bash: args.allow_bash,
    };

    if config.provider == Provider::Anthropic && config.model.contains('/') {
        bail!(
            "Model '{}' looks like an OpenRouter model (contains '/'). \
             Did you mean --provider openrouter?",
            config.model
        );
    }

    if config.post_comment {
        if config.pr_url.is_none() {
            bail!("--post-comment requires --pr to be set");
        } else if std::env::var("GITHUB_TOKEN").is_err() {
            bail!("GITHUB_TOKEN not set (required for --post-comment)");
        } else {
            eprintln!("Note: GitHub comment posting not yet implemented");
        }
    }

    if config.strict_rules {
        eprintln!("Note: --strict-rules is not yet implemented; ignoring");
    }

    let api_key = get_api_key(provider).context(match provider {
        Provider::Anthropic => {
            "ANTHROPIC_API_KEY not set. Set the environment variable:\n  \
             export ANTHROPIC_API_KEY=sk-ant-..."
        }
        Provider::OpenRouter => {
            "OPENROUTER_API_KEY not set. Set the environment variable:\n  \
             export OPENROUTER_API_KEY=sk-or-..."
        }
    })?;

    let stateless: Arc<dyn StatelessEvaluator> = match provider {
        Provider::Anthropic => Arc::new(
            AnthropicClient::new(api_key.clone())
                .map_err(|e| anyhow::anyhow!("failed to create Anthropic client: {}", e))?,
        ),
        Provider::OpenRouter => Arc::new(
            OpenRouterClient::new(api_key.clone())
                .map_err(|e| anyhow::anyhow!("failed to create OpenRouter client: {}", e))?,
        ),
    };

    let agentic: Option<Arc<dyn AgenticEvaluator>> =
        match PiAgenticEvaluator::new(api_key.clone(), provider) {
            Ok(e) => Some(Arc::new(e)),
            Err(e) => {
                eprintln!("Warning: agentic evaluator unavailable: {}", e);
                None
            }
        };

    let infra = CheckInfra::new(stateless, agentic, config.no_cache, &config.repo_root)?;

    let infra = if config.output_format == OutputFormat::Json {
        infra.with_progress(Arc::new(NullProgress))
    } else {
        infra.with_progress(Arc::from(create_progress_reporter(0)))
    };

    let report = check_pr(&infra, &config).await?;

    let mut stdout = std::io::stdout();
    print_report(
        &report,
        config.output_format,
        config.verbose,
        Some(&config.repo_root),
        &mut stdout,
        colors,
    )?;

    Ok(exit_code_for_report(&report, config.warn_as_error))
}
