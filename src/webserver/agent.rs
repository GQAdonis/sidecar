use super::agent_stream::generate_agent_stream;
use super::context_trimming::create_context_string_for_precise_contexts;
use super::types::json;
use anyhow::Context;
use futures::stream;
use futures::StreamExt;
use std::collections::HashSet;
use std::sync::Arc;

use axum::response::IntoResponse;
use axum::{extract::Query as axumQuery, Extension, Json};
/// We will invoke the agent to get the answer, we are moving to an agent based work
use serde::{Deserialize, Serialize};

use crate::agent::llm_funcs::{self, LlmClient};
use crate::agent::model::{GPT_3_5_TURBO_16K, GPT_4};
use crate::agent::prompts;
use crate::agent::types::Agent;
use crate::agent::types::AgentAction;
use crate::agent::types::CodeSpan;
use crate::agent::types::ConversationMessage;
use crate::application::application::Application;
use crate::indexes::code_snippet::CodeSnippetDocument;
use crate::repo::types::RepoRef;
use crate::webserver::context_trimming::{
    create_trimmed_context, create_viewport_context, trim_deep_context,
};

use super::types::ApiResponse;
use super::types::Result;

fn default_thread_id() -> uuid::Uuid {
    uuid::Uuid::new_v4()
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct SearchInformation {
    pub query: String,
    pub reporef: RepoRef,
    #[serde(default = "default_thread_id")]
    pub thread_id: uuid::Uuid,
}

impl ApiResponse for SearchInformation {}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct SearchResponse {
    pub query: String,
    pub answer: String,
}

impl ApiResponse for SearchResponse {}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum SearchEvents {
    SearchEvent(),
}

pub async fn search_agent(
    axumQuery(SearchInformation {
        query,
        reporef,
        thread_id,
    }): axumQuery<SearchInformation>,
    Extension(app): Extension<Application>,
) -> Result<impl IntoResponse> {
    let session_id = uuid::Uuid::new_v4();
    let llm_client = Arc::new(LlmClient::codestory_infra());
    let sql_db = app.sql.clone();
    let (sender, receiver) = tokio::sync::mpsc::channel(100);
    let action = AgentAction::Query(query.clone());
    let previous_conversation_message =
        ConversationMessage::load_from_db(sql_db.clone(), &reporef, thread_id)
            .await
            .expect("loading from db to never fail");
    let agent = Agent::prepare_for_search(
        app,
        reporef,
        session_id,
        &query,
        llm_client,
        thread_id,
        sql_db,
        previous_conversation_message,
        sender,
    );

    generate_agent_stream(agent, action, receiver).await
}

// TODO(skcd): Add write files and other things here
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct SemanticSearchQuery {
    pub query: String,
    pub reporef: RepoRef,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct SemanticSearchResponse {
    session_id: uuid::Uuid,
    query: String,
    code_spans: Vec<CodeSpan>,
}

impl ApiResponse for SemanticSearchResponse {}

pub async fn semantic_search(
    axumQuery(SemanticSearchQuery { query, reporef }): axumQuery<SemanticSearchQuery>,
    Extension(app): Extension<Application>,
) -> Result<impl IntoResponse> {
    // The best thing to do here is the following right now:
    // lexical search on the paths of the code
    // and then semantic search on the chunks we have from the file
    // we return at this point, because the latency is too high, and this is
    // okay as it is
    let session_id = uuid::Uuid::new_v4();
    let llm_client = Arc::new(LlmClient::codestory_infra());
    let conversation_id = uuid::Uuid::new_v4();
    let sql_db = app.sql.clone();
    let (sender, _) = tokio::sync::mpsc::channel(100);
    let mut agent = Agent::prepare_for_semantic_search(
        app,
        reporef,
        session_id,
        &query,
        llm_client,
        conversation_id,
        sql_db,
        vec![], // we don't have a previous conversation message here
        sender,
    );
    let code_spans = agent
        .semantic_search()
        .await
        .expect("semantic_search to not fail");
    Ok(json(SemanticSearchResponse {
        session_id,
        query,
        code_spans,
    }))
}

// Here we are experimenting with lexical search:
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct SearchQuery {
    query: String,
    repo: RepoRef,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct SearchResponseForLexicalSearch {
    code_documents: Vec<CodeSnippetDocument>,
    repo: RepoRef,
}

impl ApiResponse for SearchResponseForLexicalSearch {}

impl ApiResponse for SearchQuery {}

pub async fn lexical_search(
    axumQuery(SemanticSearchQuery { query, reporef }): axumQuery<SemanticSearchQuery>,
    Extension(app): Extension<Application>,
) -> Result<impl IntoResponse> {
    let documents = app
        .indexes
        .code_snippet
        .lexical_search(&reporef, &query, 10)
        .await
        .expect("lexical search to not fail");
    Ok(json(SearchResponseForLexicalSearch {
        code_documents: documents,
        repo: reporef,
    }))
}

// Here we are going to provide a hybrid search index which combines both the
// lexical and the semantic search together
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct HybridSearchQuery {
    query: String,
    repo: RepoRef,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct HybridSearchResponse {
    session_id: uuid::Uuid,
    query: String,
    code_spans: Vec<CodeSpan>,
}

impl ApiResponse for HybridSearchResponse {}

/// What's hybrid search? Hybrid search combines the best things about both semantic
/// and lexical search along with statistics from the git log to generate the
/// best code spans which are relevant
pub async fn hybrid_search(
    axumQuery(HybridSearchQuery { query, repo }): axumQuery<HybridSearchQuery>,
    Extension(app): Extension<Application>,
) -> Result<impl IntoResponse> {
    // Here we want to do the following:
    // - do a semantic search (normalize it to a score between 0.5 -> 1)
    // - do a lexical search (normalize it to a score between 0.5 -> 1)
    // - get statistics from the git log (normalize it to a score between 0.5 -> 1)
    // hand-waving the numbers here for whatever works for now
    // - final score -> git_log_score * 4 + lexical_search * 2.5 + semantic_search_score
    // - combine the score as following
    let session_id = uuid::Uuid::new_v4();
    let llm_client = Arc::new(LlmClient::codestory_infra());
    let conversation_id = uuid::Uuid::new_v4();
    let sql_db = app.sql.clone();
    let (sender, _) = tokio::sync::mpsc::channel(100);
    let mut agent = Agent::prepare_for_semantic_search(
        app,
        repo,
        session_id,
        &query,
        llm_client,
        conversation_id,
        sql_db,
        vec![], // we don't have a previous conversation message here
        sender,
    );
    let hybrid_search_results = agent.code_search_hybrid(&query).await.unwrap_or(vec![]);
    Ok(json(HybridSearchResponse {
        session_id: uuid::Uuid::new_v4(),
        query,
        code_spans: hybrid_search_results,
    }))
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ExplainRequest {
    query: String,
    relative_path: String,
    start_line: u64,
    end_line: u64,
    repo_ref: RepoRef,
    #[serde(default = "default_thread_id")]
    thread_id: uuid::Uuid,
}

/// We are going to handle the explain function here, but its going to be very
/// bare-bones right now. We don't give the user the option to explore or do
/// more things with the agent yet, ideal explain feature will be when the user
/// gets to explore the repository or maybe that can be a different UX like the
/// crawler
pub async fn explain(
    axumQuery(ExplainRequest {
        query,
        relative_path,
        start_line,
        end_line,
        repo_ref,
        thread_id,
    }): axumQuery<ExplainRequest>,
    Extension(app): Extension<Application>,
) -> Result<impl IntoResponse> {
    let file_content = app
        .indexes
        .file
        .get_by_path(&relative_path, &repo_ref)
        .await
        .context("file retrieval failed")?
        .context("requested file not found")?
        .content;

    let mut previous_messages =
        ConversationMessage::load_from_db(app.sql.clone(), &repo_ref, thread_id)
            .await
            .expect("loading from db to never fail");

    let snippet = file_content
        .lines()
        .skip(start_line.try_into().expect("conversion_should_not_fail"))
        .take(
            (end_line - start_line)
                .try_into()
                .expect("conversion_should_not_fail"),
        )
        .collect::<Vec<_>>()
        .join("\n");

    let mut conversation_message = ConversationMessage::explain_message(
        thread_id,
        crate::agent::types::AgentState::Explain,
        query,
    );

    let code_span = CodeSpan {
        file_path: relative_path.to_owned(),
        alias: 0,
        start_line,
        end_line,
        data: snippet,
        score: Some(1.0),
    };
    conversation_message.add_user_selected_code_span(code_span.clone());
    conversation_message.add_code_spans(code_span.clone());
    conversation_message.add_path(relative_path);

    previous_messages.push(conversation_message);

    let action = AgentAction::Answer { paths: vec![0] };

    let (sender, receiver) = tokio::sync::mpsc::channel(100);

    let session_id = uuid::Uuid::new_v4();

    let sql = app.sql.clone();

    let agent = Agent {
        application: app,
        reporef: repo_ref,
        session_id,
        conversation_messages: previous_messages,
        llm_client: Arc::new(LlmClient::codestory_infra()),
        model: GPT_4,
        sql_db: sql,
        sender,
    };

    generate_agent_stream(agent, action, receiver).await
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct FollowupChatRequest {
    pub query: String,
    pub repo_ref: RepoRef,
    pub thread_id: uuid::Uuid,
    pub deep_context: DeepContextForView,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepContextForView {
    pub repo_ref: RepoRef,
    pub precise_context: Vec<PreciseContext>,
    pub cursor_position: Option<CursorPosition>,
    pub current_view_port: Option<CurrentViewPort>,
    pub language: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionSnippet {
    pub context: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreciseContext {
    pub symbol: Symbol,
    pub hover_text: Vec<String>,
    pub definition_snippet: DefinitionSnippet,
    pub fs_file_path: String,
    pub relative_file_path: String,
    pub range: Range,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Symbol {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fuzzy_name: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorPosition {
    pub start_position: Position,
    pub end_position: Position,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentViewPort {
    pub start_position: Position,
    pub end_position: Position,
    pub relative_path: String,
    pub fs_file_path: String,
    pub text_on_screen: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Position {
    pub line: usize,
    pub character: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub start_line: usize,
    pub start_character: usize,
    pub end_line: usize,
    pub end_character: usize,
}

pub async fn followup_chat(
    Extension(app): Extension<Application>,
    Json(FollowupChatRequest {
        query,
        repo_ref,
        thread_id,
        deep_context,
    }): Json<FollowupChatRequest>,
) -> Result<impl IntoResponse> {
    let session_id = uuid::Uuid::new_v4();
    let language = deep_context.language.to_owned();
    let trimmed_context = trim_deep_context(deep_context).await;
    let trimmed_context_str = create_trimmed_context(&trimmed_context).await;
    let view_port_code_span = trimmed_context
        .current_view_port
        .as_ref()
        .map(|current_view_port| CodeSpan {
            file_path: current_view_port.fs_file_path.to_owned(),
            alias: 0,
            start_line: current_view_port
                .start_position
                .line
                .try_into()
                .expect("to work"),
            end_line: current_view_port
                .end_position
                .line
                .try_into()
                .expect("to_work"),
            data: current_view_port.text_on_screen.to_owned(),
            score: Some(1.0),
        });
    let viewport_context = create_viewport_context(
        trimmed_context.current_view_port,
        trimmed_context.current_cursor_position,
    )
    .await;
    let sql_db = app.sql.clone();
    // Now we check if we have any previous messages, if we do we have to signal
    // that to the agent that this could be a followup question, and if we don't
    // know about that then its totally fine
    let mut conversation_message = ConversationMessage::general_question(
        thread_id,
        crate::agent::types::AgentState::ViewPort,
        query.to_owned(),
    );
    let mut previous_messages =
        ConversationMessage::load_from_db(sql_db.clone(), &repo_ref, thread_id)
            .await
            .expect("loading from db to never fail");
    if let Some(previous_message) = previous_messages.last() {
        previous_message.get_paths().iter().for_each(|path| {
            conversation_message.add_path(path.to_owned());
        });
        previous_message.code_spans().iter().for_each(|code_span| {
            conversation_message.add_code_spans(code_span.clone());
        });
        previous_message
            .user_selected_code_spans()
            .iter()
            .for_each(|code_span| {
                conversation_message.add_user_selected_code_span(code_span.clone())
            });
    }
    let (sender, receiver) = tokio::sync::mpsc::channel(100);

    let llm_client = Arc::new(LlmClient::codestory_infra());
    let goto_definitions_required =
        prompts::extract_goto_definition_symbols_from_snippet(&language);
    let (message_sender, message_receiver) = tokio::sync::mpsc::unbounded_channel();
    let response = dbg!(
        llm_client
            .stream_response(
                crate::agent::llm_funcs::llm::OpenAIModel::GPT4,
                vec![
                    llm_funcs::llm::Message::system(
                        &prompts::extract_goto_definition_symbols_from_snippet(&language),
                    ),
                    llm_funcs::llm::Message::user(&viewport_context),
                ],
                None,
                0.0,
                None,
                message_sender,
            )
            .await
    );
    let required_definition_strings = dbg!(response
        .unwrap_or_default()
        .split(",")
        .map(|s| s.trim().to_owned())
        .collect::<HashSet<_>>());
    // let definitions_required: Vec<_> =
    //     stream::iter(trimmed_context_str.into_iter().map(|trimmed_context| {
    //         (
    //             trimmed_context,
    //             viewport_context.to_owned(),
    //             llm_client.clone(),
    //             query.to_owned(),
    //         )
    //     }))
    //     .filter_map(
    //         |(trimmed_context_str, view_port_context, llm_client, query)| async move {
    //             // here we will call out the prompt with gpt3 and see what happens
    //             // after that
    //             let messages = vec![llm_funcs::llm::Message::system(
    //                 &prompts::definition_snippet_required(
    //                     &view_port_context,
    //                     &trimmed_context_str,
    //                     &query,
    //                 ),
    //             )];
    //             let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
    //             let response = llm_client
    //                 .stream_response(
    //                     crate::agent::llm_funcs::llm::OpenAIModel::GPT4,
    //                     messages,
    //                     None,
    //                     0.2,
    //                     None,
    //                     sender,
    //                 )
    //                 .await;
    //             dbg!(&response);
    //             let final_response = response.unwrap_or("NO".to_owned());
    //             if final_response.to_lowercase() == "no" {
    //                 None
    //             } else {
    //                 Some(futures::future::ready(trimmed_context_str))
    //             }
    //         },
    //     )
    //     .buffer_unordered(10)
    //     .collect::<Vec<_>>()
    //     .await;

    let definitions_required = trimmed_context
        .precise_context_map
        .into_iter()
        .filter_map(|(_, precise_contexts)| {
            // check if the trimmed context string has any of the required strings,
            // if it does we should include it otherwise we can exclude it
            if precise_contexts.iter().any(|context| {
                context
                    .symbol
                    .fuzzy_name
                    .as_ref()
                    .map(|fuzzy_name| required_definition_strings.contains(fuzzy_name))
                    .unwrap_or_default()
            }) {
                Some(create_context_string_for_precise_contexts(precise_contexts))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    // we will grab the required definitions from the response of the LLM here
    dbg!(&definitions_required);

    // very very hacky and very bad, but oh well we will figure out a way to
    // re-rank things properly
    if let Some(view_port_code_span) = view_port_code_span {
        conversation_message.add_user_selected_code_span(view_port_code_span);
    }
    conversation_message.add_definitions_interested_in(definitions_required);
    previous_messages.push(conversation_message);
    let agent = Agent::prepare_for_followup(
        app,
        repo_ref,
        session_id,
        llm_client.clone(),
        sql_db,
        previous_messages,
        sender,
    );

    // If this is a followup, right now we don't take in any additional context,
    // but only use the one from our previous conversation
    let action = AgentAction::Answer { paths: vec![] };

    generate_agent_stream(agent, action, receiver).await
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GotoDefinitionSymbolsRequest {
    pub code_snippet: String,
    pub language: String,
    pub repo_ref: RepoRef,
    pub thread_id: uuid::Uuid,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GotoDefinitionSymbolsResponse {
    symbols: Vec<String>,
}

impl ApiResponse for GotoDefinitionSymbolsResponse {}

pub async fn go_to_definition_symbols(
    Extension(app): Extension<Application>,
    Json(GotoDefinitionSymbolsRequest {
        code_snippet,
        language,
        repo_ref,
        thread_id,
    }): Json<GotoDefinitionSymbolsRequest>,
) -> Result<impl IntoResponse> {
    dbg!("we are over here in goto_definition_symbol");
    let sql_db = app.sql.clone();
    let agent = Agent {
        application: app,
        reporef: repo_ref,
        session_id: uuid::Uuid::new_v4(),
        conversation_messages: vec![],
        llm_client: Arc::new(LlmClient::codestory_infra()),
        model: GPT_3_5_TURBO_16K,
        sql_db,
        sender: tokio::sync::mpsc::channel(100).0,
    };
    let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
    Ok(json(GotoDefinitionSymbolsResponse {
        symbols: agent
            .goto_definition_symbols(&code_snippet, &language, sender)
            .await
            .expect("goto_definition_symbols to not fail"),
    }))
}

#[cfg(test)]
mod tests {
    use super::FollowupChatRequest;
    use serde_json;

    #[test]
    fn test_parsing() {
        let input_string = r#"
        {"repo_ref":"local//Users/skcd/scratch/website","query":"whats happenign here","thread_id":"7cb05252-1bb8-4d5e-a942-621ab5d5e114","deep_context":{"repoRef":"local//Users/skcd/scratch/website","preciseContext":[{"symbol":{"fuzzyName":"Author"},"fsFilePath":"/Users/skcd/scratch/website/interfaces/author.ts","relativeFilePath":"interfaces/author.ts","range":{"startLine":0,"startCharacter":0,"endLine":6,"endCharacter":1},"hoverText":["\n```typescript\n(alias) type Author = {\n    name: string;\n    picture: string;\n    twitter: string;\n    linkedin: string;\n    github: string;\n}\nimport Author\n```\n",""],"definitionSnippet":"type Author = {\n  name: string\n  picture: string\n  twitter: string\n  linkedin: string\n  github: string\n}"}],"cursorPosition":{"startPosition":{"line":16,"character":0},"endPosition":{"line":16,"character":0}},"currentViewPort":{"startPosition":{"line":0,"character":0},"endPosition":{"line":16,"character":0},"fsFilePath":"/Users/skcd/scratch/website/interfaces/post.ts","relativePath":"interfaces/post.ts","textOnScreen":"import type Author from './author'\n\ntype PostType = {\n  slug: string\n  title: string\n  date: string\n  coverImage: string\n  author: Author\n  excerpt: string\n  ogImage: {\n    url: string\n  }\n  content: string\n}\n\nexport default PostType\n"}}}
        "#;
        let parsed_response = serde_json::from_str::<FollowupChatRequest>(&input_string);
        assert!(parsed_response.is_ok());
    }
}
