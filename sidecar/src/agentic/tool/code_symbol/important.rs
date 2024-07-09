//! Here we grab the important symbols which we are going to edit or follow further
//! and figure out what we should be doing next
//! At each step we are going to focus on the current symbol and keep adding the
//! rest ones to our history and keep them, this is how agents are going to look like
//! These are like state-machines which are holding memory and moving forward and collaborating.

use async_trait::async_trait;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use llm_client::{
    broker::LLMBroker,
    clients::types::LLMType,
    provider::{LLMProvider, LLMProviderAPIKeys},
};

use crate::{
    agentic::{
        symbol::identifier::LLMProperties,
        tool::{
            errors::ToolError,
            input::ToolInput,
            output::ToolOutput,
            r#type::{Tool, ToolType},
        },
    },
    chunking::text_document::Range,
    user_context::types::UserContext,
};

use super::{
    models::anthropic::{
        AnthropicCodeSymbolImportant, CodeSymbolShouldAskQuestionsResponse,
        CodeSymbolToAskQuestionsResponse, ProbeNextSymbol,
    },
    types::CodeSymbolError,
};

use crate::chunking::languages::TSLanguageParsing;

pub struct CodeSymbolImportantBroker {
    llms: HashMap<LLMType, Box<dyn CodeSymbolImportant + Send + Sync>>,
}

impl CodeSymbolImportantBroker {
    pub fn new(llm_client: Arc<LLMBroker>, fail_over_llm: LLMProperties) -> Self {
        let mut llms: HashMap<LLMType, Box<dyn CodeSymbolImportant + Send + Sync>> = HashMap::new();
        llms.insert(
            LLMType::ClaudeHaiku,
            Box::new(AnthropicCodeSymbolImportant::new(
                llm_client.clone(),
                fail_over_llm.clone(),
            )),
        );
        llms.insert(
            LLMType::ClaudeSonnet,
            Box::new(AnthropicCodeSymbolImportant::new(
                llm_client.clone(),
                fail_over_llm.clone(),
            )),
        );
        llms.insert(
            LLMType::ClaudeOpus,
            Box::new(AnthropicCodeSymbolImportant::new(
                llm_client.clone(),
                fail_over_llm.clone(),
            )),
        );
        llms.insert(
            LLMType::Gpt4O,
            Box::new(AnthropicCodeSymbolImportant::new(
                llm_client.clone(),
                fail_over_llm.clone(),
            )),
        );
        llms.insert(
            LLMType::GeminiPro,
            Box::new(AnthropicCodeSymbolImportant::new(
                llm_client.clone(),
                fail_over_llm.clone(),
            )),
        );
        llms.insert(
            LLMType::GeminiProFlash,
            Box::new(AnthropicCodeSymbolImportant::new(
                llm_client.clone(),
                fail_over_llm,
            )),
        );
        Self { llms }
    }
}

#[async_trait]
impl Tool for CodeSymbolImportantBroker {
    async fn invoke(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        // PS: This is getting out of hand
        if input.is_probe_summarization_request() {
            let context = input.probe_summarization_request()?;
            if let Some(implementation) = self.llms.get(context.llm()) {
                return implementation
                    .probe_summarize_answer(context)
                    .await
                    .map(|response| ToolOutput::probe_summarization_result(response))
                    .map_err(|e| ToolError::CodeSymbolError(e));
            }
        } else if input.is_probe_follow_along_symbol_request() {
            let context = input.probe_follow_along_symbol()?;
            if let Some(implementation) = self.llms.get(context.llm()) {
                return implementation
                    .should_probe_follow_along_symbol_request(context)
                    .await
                    .map(|response| ToolOutput::probe_follow_along_symbol(response))
                    .map_err(|e| ToolError::CodeSymbolError(e));
            }
        } else if input.is_probe_possible_request() {
            let context = input.probe_possible_request()?;
            if let Some(implementation) = self.llms.get(&context.model()) {
                return implementation
                    .should_probe_question_request(context)
                    .await
                    .map(|response| ToolOutput::probe_possible(response))
                    .map_err(|e| ToolError::CodeSymbolError(e));
            }
        } else if input.is_probe_question() {
            let context = input.probe_question_request()?;
            if let Some(implementation) = self.llms.get(&context.model()) {
                return implementation
                    .symbols_to_probe_questions(context)
                    .await
                    .map(|response| ToolOutput::ProbeQuestion(response))
                    .map_err(|e| ToolError::CodeSymbolError(e));
            }
        } else if input.is_utility_code_search() {
            let context = input.utility_code_search()?;
            if let Some(implementation) = self.llms.get(&context.model()) {
                return implementation
                    .gather_utility_symbols(context)
                    .await
                    .map(|response| ToolOutput::utility_code_symbols(response))
                    .map_err(|e| ToolError::CodeSymbolError(e));
            }
        } else {
            let context = input.code_symbol_search();
            if let Ok(context) = context {
                match context {
                    either::Left(context) => {
                        if let Some(implementation) = self.llms.get(context.model()) {
                            return implementation
                                .get_important_symbols(context)
                                .await
                                .map(|response| ToolOutput::important_symbols(response))
                                .map_err(|e| ToolError::CodeSymbolError(e));
                        }
                    }
                    either::Right(context) => {
                        if let Some(implementation) = self.llms.get(context.model()) {
                            return implementation
                                .context_wide_search(context)
                                .await
                                .map(|response| ToolOutput::important_symbols(response))
                                .map_err(|e| ToolError::CodeSymbolError(e));
                        }
                    }
                };
            }
        }
        Err(ToolError::WrongToolInput(ToolType::RequestImportantSymbols))
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CodeSymbolImportantWideSearch {
    user_context: UserContext,
    user_query: String,
    llm_type: LLMType,
    llm_provider: LLMProvider,
    api_key: LLMProviderAPIKeys,
    file_extension_filters: HashSet<String>,
}

impl CodeSymbolImportantWideSearch {
    pub fn new(
        user_context: UserContext,
        user_query: String,
        llm_type: LLMType,
        llm_provider: LLMProvider,
        api_key: LLMProviderAPIKeys,
    ) -> Self {
        Self {
            user_context,
            user_query,
            llm_type,
            llm_provider,
            api_key,
            file_extension_filters: Default::default(),
        }
    }

    pub fn set_file_extension_fitler(mut self, file_extension: String) -> Self {
        self.file_extension_filters.insert(file_extension);
        self
    }

    pub fn file_extension_filters(&self) -> HashSet<String> {
        self.file_extension_filters.clone()
    }

    pub fn user_query(&self) -> &str {
        &self.user_query
    }

    pub fn api_key(&self) -> LLMProviderAPIKeys {
        self.api_key.clone()
    }

    pub fn llm_provider(&self) -> LLMProvider {
        self.llm_provider.clone()
    }

    pub fn model(&self) -> &LLMType {
        &self.llm_type
    }

    pub fn user_context(&self) -> &UserContext {
        &self.user_context
    }

    pub fn remove_user_context(self) -> UserContext {
        self.user_context
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CodeSymbolUtilityRequest {
    user_query: String,
    definitions_alredy_present: Vec<String>,
    fs_file_path: String,
    fs_file_content: String,
    selection_range: Range,
    language: String,
    llm_type: LLMType,
    llm_provider: LLMProvider,
    api_key: LLMProviderAPIKeys,
    user_context: UserContext,
}

impl CodeSymbolUtilityRequest {
    pub fn new(
        user_query: String,
        definitions_alredy_present: Vec<String>,
        fs_file_path: String,
        fs_file_content: String,
        selection_range: Range,
        language: String,
        llm_type: LLMType,
        llm_provider: LLMProvider,
        api_key: LLMProviderAPIKeys,
        user_context: UserContext,
    ) -> Self {
        Self {
            user_query,
            definitions_alredy_present,
            fs_file_content,
            fs_file_path,
            selection_range,
            language,
            llm_provider,
            llm_type,
            api_key,
            user_context,
        }
    }

    pub fn definitions(&self) -> &[String] {
        self.definitions_alredy_present.as_slice()
    }

    pub fn selection_range(&self) -> &Range {
        &self.selection_range
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn fs_file_path(&self) -> &str {
        &self.fs_file_path
    }

    pub fn file_content(&self) -> &str {
        &self.fs_file_content
    }

    pub fn user_query(&self) -> &str {
        &self.user_query
    }

    pub fn model(&self) -> LLMType {
        self.llm_type.clone()
    }

    pub fn provider(&self) -> LLMProvider {
        self.llm_provider.clone()
    }

    pub fn api_key(&self) -> LLMProviderAPIKeys {
        self.api_key.clone()
    }

    pub fn user_context(self) -> UserContext {
        self.user_context
    }
}

/// Contains the probing results from a sub-symbol
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeSubSymbolProbingResult {
    symbol_name: String,
    fs_file_path: String,
    probing_results: Vec<String>,
    content: String,
}

impl CodeSubSymbolProbingResult {
    pub fn new(
        symbol_name: String,
        fs_file_path: String,
        probing_results: Vec<String>,
        content: String,
    ) -> Self {
        Self {
            symbol_name,
            fs_file_path,
            probing_results,
            content,
        }
    }

    pub fn to_xml(&self) -> String {
        let symbol_name = &self.symbol_name;
        let file_path = &self.fs_file_path;
        let probing_results = self.probing_results.join("\n");
        let content = &self.content;
        format!(
            r#"<symbol>
<name>
{symbol_name}
</name>
<file_path>
{file_path}
</file_path>
<content>
{content}
</content>
<probing_results>
{probing_results}
</probing_results>
</symbol>"#
        )
    }
}

/// This request is used to answer the probing request in total after we have
/// explored the current node properly.
/// We do many explorations from the current symbol and we summarize our answers
/// here
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeSymbolProbingSummarize {
    query: String,
    history: String,
    symbol_identifier: String,
    symbol_outline: String,
    fs_file_path: String,
    probing_results: Vec<CodeSubSymbolProbingResult>,
    llm: LLMType,
    provider: LLMProvider,
    api_keys: LLMProviderAPIKeys,
}

impl CodeSymbolProbingSummarize {
    pub fn new(
        query: String,
        history: String,
        symbol_identifier: String,
        symbol_outline: String,
        fs_file_path: String,
        probing_results: Vec<CodeSubSymbolProbingResult>,
        llm: LLMType,
        provider: LLMProvider,
        api_keys: LLMProviderAPIKeys,
    ) -> Self {
        Self {
            query,
            history,
            symbol_identifier,
            symbol_outline,
            fs_file_path,
            probing_results,
            llm,
            provider,
            api_keys,
        }
    }

    pub fn symbol_probing_results(&self) -> String {
        self.probing_results
            .iter()
            .map(|probing_result| probing_result.to_xml())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn symbol_outline(&self) -> &str {
        &self.symbol_outline
    }

    pub fn user_query(&self) -> &str {
        &self.query
    }

    pub fn history(&self) -> &str {
        &self.history
    }

    pub fn symbol_identifier(&self) -> &str {
        &self.symbol_identifier
    }

    pub fn fs_file_path(&self) -> &str {
        &self.fs_file_path
    }

    pub fn probing_results(&self) -> &[CodeSubSymbolProbingResult] {
        self.probing_results.as_slice()
    }

    pub fn llm(&self) -> &LLMType {
        &self.llm
    }

    pub fn provider(&self) -> &LLMProvider {
        &self.provider
    }

    pub fn api_keys(&self) -> &LLMProviderAPIKeys {
        &self.api_keys
    }
}

/// This requests determines if we have to follow the next code symbol
/// or if we have enough information at this point to stop and answer the user
/// query, or we are on the wrong trail or we need to keep probing
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeSymbolFollowAlongForProbing {
    history: String,
    symbol_identifier: String,
    fs_file_path: String,
    language: String,
    next_symbol_names: Vec<String>,
    next_symbol_outlines: Vec<String>,
    // This is for the current file we are interested in
    code_above: Option<String>,
    code_below: Option<String>,
    code_in_selection: String,
    llm_type: LLMType,
    provider: LLMProvider,
    api_key: LLMProviderAPIKeys,
    query: String,
    next_symbol_link: String,
}

impl CodeSymbolFollowAlongForProbing {
    pub fn new(
        history: String,
        symbol_identifier: String,
        fs_file_path: String,
        language: String,
        next_symbol_names: Vec<String>,
        next_symbol_outlines: Vec<String>,
        code_above: Option<String>,
        code_below: Option<String>,
        code_in_selection: String,
        llm_type: LLMType,
        provider: LLMProvider,
        api_key: LLMProviderAPIKeys,
        query: String,
        next_symbol_link: String,
    ) -> Self {
        Self {
            history,
            symbol_identifier,
            fs_file_path,
            language,
            next_symbol_names,
            next_symbol_outlines,
            code_above,
            code_below,
            code_in_selection,
            llm_type,
            provider,
            api_key,
            query,
            next_symbol_link,
        }
    }

    pub fn next_symbol_link(&self) -> &str {
        &self.next_symbol_link
    }

    pub fn user_query(&self) -> &str {
        &self.query
    }

    pub fn code_above(&self) -> Option<String> {
        self.code_above.clone()
    }

    pub fn code_below(&self) -> Option<String> {
        self.code_below.clone()
    }

    pub fn code_in_selection(&self) -> &str {
        &self.code_in_selection
    }

    pub fn file_path(&self) -> &str {
        &self.fs_file_path
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn next_symbol_names(&self) -> &[String] {
        self.next_symbol_names.as_slice()
    }

    pub fn next_symbol_outline(&self) -> &[String] {
        self.next_symbol_outlines.as_slice()
    }

    pub fn llm(&self) -> &LLMType {
        &self.llm_type
    }

    pub fn llm_provider(&self) -> &LLMProvider {
        &self.provider
    }

    pub fn api_keys(&self) -> &LLMProviderAPIKeys {
        &self.api_key
    }

    pub fn history(&self) -> &str {
        &self.history
    }
}

/// This request will give us code symbols and additional questions
/// we want to ask them before making our edits
/// This way we can ensure that the world moves to the state we are interested in
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeSymbolToAskQuestionsRequest {
    history: String,
    symbol_identifier: String,
    fs_file_path: String,
    language: String,
    extra_data: String,
    code_above: Option<String>,
    code_below: Option<String>,
    code_in_selection: String,
    llm_type: LLMType,
    provider: LLMProvider,
    api_key: LLMProviderAPIKeys,
    query: String,
}

impl CodeSymbolToAskQuestionsRequest {
    pub fn new(
        history: String,
        symbol_identifier: String,
        fs_file_path: String,
        language: String,
        extra_data: String,
        code_above: Option<String>,
        code_below: Option<String>,
        code_in_selection: String,
        llm_type: LLMType,
        provider: LLMProvider,
        api_key: LLMProviderAPIKeys,
        query: String,
    ) -> Self {
        Self {
            history,
            symbol_identifier,
            fs_file_path,
            language,
            extra_data,
            code_above,
            code_below,
            code_in_selection,
            llm_type,
            provider,
            api_key,
            query,
        }
    }

    pub fn api_key(&self) -> &LLMProviderAPIKeys {
        &self.api_key
    }

    pub fn provider(&self) -> &LLMProvider {
        &self.provider
    }

    pub fn symbol_identifier(&self) -> &str {
        &self.symbol_identifier
    }

    pub fn history(&self) -> &str {
        &self.history
    }

    pub fn user_query(&self) -> &str {
        &self.query
    }

    pub fn extra_data(&self) -> &str {
        &self.extra_data
    }

    pub fn fs_file_path(&self) -> &str {
        &self.fs_file_path
    }

    pub fn code_above(&self) -> Option<String> {
        self.code_above.clone()
    }

    pub fn code_below(&self) -> Option<String> {
        self.code_below.clone()
    }

    pub fn code_in_selection(&self) -> &str {
        &self.code_in_selection
    }

    pub fn model(&self) -> &LLMType {
        &self.llm_type
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CodeSymbolImportantRequest {
    // if we have any symbol identifier here which we are focussing on, we keep
    // track of that here, if there is no history then we do not care about it.
    symbol_identifier: Option<String>,
    // history here consists of the symbols which we have followed to get to this
    // place
    history: Vec<String>,
    fs_file_path: String,
    fs_file_content: String,
    selection_range: Range,
    language: String,
    llm_type: LLMType,
    llm_provider: LLMProvider,
    api_key: LLMProviderAPIKeys,
    // this at the start will be the user query
    query: String,
}

impl CodeSymbolImportantRequest {
    pub fn new(
        symbol_identifier: Option<String>,
        history: Vec<String>,
        fs_file_path: String,
        fs_file_content: String,
        selection_range: Range,
        llm_type: LLMType,
        llm_provider: LLMProvider,
        api_key: LLMProviderAPIKeys,
        language: String,
        query: String,
    ) -> Self {
        Self {
            symbol_identifier,
            history,
            fs_file_path,
            fs_file_content,
            selection_range,
            llm_type,
            llm_provider,
            api_key,
            query,
            language,
        }
    }

    pub fn symbol_identifier(&self) -> Option<&str> {
        self.symbol_identifier.as_deref()
    }

    pub fn model(&self) -> &LLMType {
        &self.llm_type
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn file_path(&self) -> &str {
        &self.fs_file_path
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn content(&self) -> &str {
        &self.fs_file_content
    }

    pub fn range(&self) -> &Range {
        &self.selection_range
    }

    pub fn api_key(&self) -> &LLMProviderAPIKeys {
        &self.api_key
    }

    pub fn provider(&self) -> &LLMProvider {
        &self.llm_provider
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeSymbolWithThinking {
    code_symbol: String,
    thinking: String,
    file_path: String,
}

impl CodeSymbolWithThinking {
    pub fn new(code_symbol: String, thinking: String, file_path: String) -> Self {
        Self {
            code_symbol,
            thinking,
            file_path,
        }
    }

    pub fn fs_prefix(mut self, fs_prefix: &str) -> Self {
        self.file_path = fs_prefix.to_owned() + "/" + &self.file_path;
        self
    }

    pub fn code_symbol(&self) -> &str {
        &self.code_symbol
    }

    pub fn thinking(&self) -> &str {
        &self.thinking
    }

    pub fn file_path(&self) -> &str {
        &self.file_path
    }

    /// If the symbol name consists of a.b.c kind of format we want to grab
    /// just the a instead of the whole string since we always work on the
    /// top level symbol
    pub fn fix_symbol_name(self, ts_parsing: Arc<TSLanguageParsing>) -> Self {
        if self.file_path().ends_with("py") {
            if self.code_symbol.contains(".") {
                let language = "python";
                let ts_language_config = ts_parsing
                    .for_lang(language)
                    .expect("language config to be present");

                if let Some(range) =
                    ts_language_config.generate_object_qualifier(self.code_symbol.as_bytes())
                {
                    let object_qualifier = &self.code_symbol[range.start_byte()..range.end_byte()];
                    Self {
                        code_symbol: object_qualifier.to_string(),
                        thinking: self.thinking,
                        file_path: self.file_path,
                    }
                } else {
                    let mut code_symbol_parts = self.code_symbol.split(".").collect::<Vec<_>>();
                    if code_symbol_parts.is_empty() {
                        self
                    } else {
                        Self {
                            code_symbol: code_symbol_parts.remove(0).to_owned(),
                            thinking: self.thinking,
                            file_path: self.file_path,
                        }
                    }
                }
            } else {
                self
            }
        } else if self.file_path().ends_with("rs") {
            // we get inputs in the format: "struct::function_inside_struct"
            // we obviously know at this point that the symbol we are referring to is "function_inside_struct" in
            // "struct"
            if self.code_symbol.contains("::") {
                let language = "rust";
                let ts_language_config = ts_parsing
                    .for_lang(language)
                    .expect("language config to be present");

                if let Some(range) =
                    ts_language_config.generate_object_qualifier(self.code_symbol.as_bytes())
                {
                    let object_qualifier = &self.code_symbol[range.start_byte()..range.end_byte()];
                    Self {
                        code_symbol: object_qualifier.to_string(),
                        thinking: self.thinking,
                        file_path: self.file_path,
                    }
                } else {
                    let mut code_symbol_parts = self.code_symbol.split("::").collect::<Vec<_>>();
                    if code_symbol_parts.is_empty() {
                        self
                    } else {
                        Self {
                            code_symbol: code_symbol_parts.remove(0).to_owned(),
                            thinking: self.thinking,
                            file_path: self.file_path,
                        }
                    }
                }
            } else {
                self
            }
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeSymbolWithSteps {
    code_symbol: String,
    steps: Vec<String>,
    is_new: bool,
    file_path: String,
}

impl CodeSymbolWithSteps {
    pub fn new(code_symbol: String, steps: Vec<String>, is_new: bool, file_path: String) -> Self {
        Self {
            code_symbol,
            steps,
            is_new,
            file_path,
        }
    }

    pub fn fs_prefix(mut self, fs_prefix: &str) -> Self {
        self.file_path = fs_prefix.to_owned() + "/" + &self.file_path;
        self
    }

    pub fn code_symbol(&self) -> &str {
        &self.code_symbol
    }

    pub fn steps(&self) -> &[String] {
        self.steps.as_slice()
    }

    pub fn is_new(&self) -> bool {
        self.is_new
    }

    pub fn file_path(&self) -> &str {
        &self.file_path
    }

    /// If the symbol name consists of a.b.c kind of format we want to grab
    /// just the a instead of the whole string since we always work on the
    /// top level symbol
    pub fn fix_symbol_name(self, ts_parsing: Arc<TSLanguageParsing>) -> Self {
        if self.file_path().ends_with("py") {
            let language = "python";
            let ts_language_config = ts_parsing
                .for_lang(language)
                .expect("language config to be present");

            if let Some(range) =
                ts_language_config.generate_object_qualifier(self.code_symbol.as_bytes())
            {
                let object_qualifier = &self.code_symbol[range.start_byte()..range.end_byte()];
                Self {
                    code_symbol: object_qualifier.to_string(),
                    file_path: self.file_path,
                    steps: self.steps,
                    is_new: self.is_new,
                }
            } else {
                let mut code_symbol_parts = self.code_symbol.split(".").collect::<Vec<_>>();
                if code_symbol_parts.is_empty() {
                    self
                } else {
                    Self {
                        code_symbol: code_symbol_parts.remove(0).to_owned(),
                        steps: self.steps,
                        is_new: self.is_new,
                        file_path: self.file_path,
                    }
                }
            }
        } else if self.file_path().ends_with("rs") {
            // we get inputs in the format: "struct::function_inside_struct"
            // we obviously know at this point that the symbol we are referring to is "function_inside_struct" in
            // "struct"
            if self.code_symbol.contains("::") {
                let language = "rust";
                let ts_language_config = ts_parsing
                    .for_lang(language)
                    .expect("language config to be present");

                if let Some(range) =
                    ts_language_config.generate_object_qualifier(self.code_symbol.as_bytes())
                {
                    let object_qualifier = &self.code_symbol[range.start_byte()..range.end_byte()];
                    Self {
                        code_symbol: object_qualifier.to_string(),
                        steps: self.steps,
                        is_new: self.is_new,
                        file_path: self.file_path,
                    }
                } else {
                    let mut code_symbol_parts = self.code_symbol.split("::").collect::<Vec<_>>();
                    if code_symbol_parts.is_empty() {
                        self
                    } else {
                        Self {
                            code_symbol: code_symbol_parts.remove(0).to_owned(),
                            steps: self.steps,
                            is_new: self.is_new,
                            file_path: self.file_path,
                        }
                    }
                }
            } else {
                self
            }
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeSymbolImportantResponse {
    symbols: Vec<CodeSymbolWithThinking>,
    ordered_symbols: Vec<CodeSymbolWithSteps>,
}

impl CodeSymbolImportantResponse {
    pub fn new(
        symbols: Vec<CodeSymbolWithThinking>,
        ordered_symbols: Vec<CodeSymbolWithSteps>,
    ) -> Self {
        Self {
            symbols,
            ordered_symbols,
        }
    }

    /// The fix for symbol name here is that we could have symbols come in
    /// the form of a.b.c etc
    /// so we want to parse them as just a instead of a.b.c
    /// this way we can ensure that we find the right symbol always
    pub fn fix_symbol_names(self, ts_parsing: Arc<TSLanguageParsing>) -> Self {
        let symbols = self.symbols;
        let ordered_symbols = self.ordered_symbols;
        Self {
            symbols: symbols
                .into_iter()
                .map(|symbol| symbol.fix_symbol_name(ts_parsing.clone()))
                .collect::<Vec<_>>(),
            ordered_symbols: ordered_symbols
                .into_iter()
                .map(|symbol| symbol.fix_symbol_name(ts_parsing.clone()))
                .collect::<Vec<_>>(),
        }
    }

    pub fn add_fs_prefix(self, fs_prefix: String) -> Self {
        let symbols = self.symbols;
        let ordered_symbols = self.ordered_symbols;
        let mod_symbols = symbols
            .into_iter()
            .map(|symbol| symbol.fs_prefix(&fs_prefix))
            .collect::<Vec<_>>();
        let mod_ordered_symbols = ordered_symbols
            .into_iter()
            .map(|ordered_symbol| ordered_symbol.fs_prefix(&fs_prefix))
            .collect::<Vec<_>>();
        Self {
            symbols: mod_symbols,
            ordered_symbols: mod_ordered_symbols,
        }
    }

    pub fn symbols(&self) -> &[CodeSymbolWithThinking] {
        self.symbols.as_slice()
    }

    pub fn remove_symbols(self) -> Vec<CodeSymbolWithThinking> {
        self.symbols
    }

    pub fn ordered_symbols(&self) -> &[CodeSymbolWithSteps] {
        self.ordered_symbols.as_slice()
    }

    pub fn ordered_symbols_to_plan(&self) -> String {
        // We try to create a shallow plan here for our agents using the initial
        // plan, this will help them stay in place and follow the initial logic
        // which we have generated
        self.ordered_symbols
            .iter()
            .enumerate()
            .map(|(idx, ordered_symbol)| {
                let idx = idx + 1;
                let code_symbol = &ordered_symbol.code_symbol;
                let fs_file_path = &ordered_symbol.file_path();
                let thinking = ordered_symbol.steps().join("\n");
                format!(
                    "<step id = {idx}>
<code_symbol>{code_symbol}</code_symbol>
<file_path>{fs_file_path}</file_path>
<high_level_plan>
{thinking}
</high_level_plan>
</step>"
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[async_trait]
pub trait CodeSymbolImportant {
    async fn get_important_symbols(
        &self,
        code_symbols: CodeSymbolImportantRequest,
    ) -> Result<CodeSymbolImportantResponse, CodeSymbolError>;

    async fn context_wide_search(
        &self,
        context_wide_search: CodeSymbolImportantWideSearch,
    ) -> Result<CodeSymbolImportantResponse, CodeSymbolError>;

    async fn gather_utility_symbols(
        &self,
        utility_symbol_request: CodeSymbolUtilityRequest,
    ) -> Result<CodeSymbolImportantResponse, CodeSymbolError>;

    // Use this to ask probing questions to the various identifiers
    async fn symbols_to_probe_questions(
        &self,
        request: CodeSymbolToAskQuestionsRequest,
    ) -> Result<CodeSymbolToAskQuestionsResponse, CodeSymbolError>;

    // asks if we want to get more probing question to the snippet we are interested
    // in
    async fn should_probe_question_request(
        &self,
        request: CodeSymbolToAskQuestionsRequest,
    ) -> Result<CodeSymbolShouldAskQuestionsResponse, CodeSymbolError>;

    /// figures out if the next symbol is necessary to be probed or we have a possible
    /// answer or have hit a dead-end and its not worth following this trail anymore
    async fn should_probe_follow_along_symbol_request(
        &self,
        request: CodeSymbolFollowAlongForProbing,
    ) -> Result<ProbeNextSymbol, CodeSymbolError>;

    /// summarizes the results from the different probes running in the background
    async fn probe_summarize_answer(
        &self,
        request: CodeSymbolProbingSummarize,
    ) -> Result<String, CodeSymbolError>;
}

// implement passing in just the user context and getting the data back
// we have to implement a wider search over here and grab all the symbols and
// then further refine it and set out agents to work on them
// let's see how that works out (would be interesting)

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::chunking::languages::TSLanguageParsing;

    use super::{CodeSymbolImportantResponse, CodeSymbolWithThinking};

    #[test]
    fn fixing_code_symbols_work() {
        let mut response = CodeSymbolImportantResponse::new(
            vec![CodeSymbolWithThinking::new(
                "LLMBroker::new".to_owned(),
                "".to_owned(),
                "/tmp/something.rs".to_owned(),
            )],
            vec![],
        );
        response = response.fix_symbol_names(Arc::new(TSLanguageParsing::init()));
        assert_eq!(response.symbols.remove(0).code_symbol, "LLMBroker");
    }
}
