/// We want to parse the python language properly and the language config
/// for it
use crate::chunking::languages::TSLanguageConfig;

pub fn python_language_config() -> TSLanguageConfig {
    TSLanguageConfig {
        language_ids: &["Python", "python"],
        file_extensions: &["py"],
        grammar: tree_sitter_python::language,
        namespaces: vec![vec!["class", "function", "parameter", "variable"]
            .into_iter()
            .map(|s| s.to_owned())
            .collect()],
        documentation_query: vec!["(expression_statement (string) @comment)".to_owned()],
        function_query: vec![
            "[
                (function_definition
                    name: (identifier) @identifier
                    parameters: (parameters
                        (typed_parameter
                        	  (identifier) @parameters.identifier
                              type: (type) @parameter.type
                         )
                         (identifier) @parameters.identifier
                    ) @parameters
                    return_type: (type)? @return_type
                    body: (block
                        (expression_statement (string))? @docstring
                        (expression_statement
                          (assignment
                            left: (identifier) @variable.name
                            type: (type)? @variable.type
                          )
                        )*
                      ) @function.body)
                (assignment
                    left: (identifier) @identifier
                    type: (type) @parameters
                    right: (lambda) @body)
            ] @function"
                .to_owned(),
            "(ERROR (\"def\" (identifier) (parameters))) @function".to_owned(),
        ],
        construct_types: vec!["module", "class_definition", "function_definition"]
            .into_iter()
            .map(|s| s.to_owned())
            .collect(),
        expression_statements: vec!["expression_statement".to_owned()],
        class_query: vec![
            "(class_definition name: (identifier) @identifier) @class_declaration".to_owned(),
        ],
        r#type_query: vec![],
        namespace_types: vec![],
        hoverable_query: r#"
        (identifier) @hoverable
        "#
        .to_owned(),
        comment_prefix: "#".to_owned(),
        end_of_line: None,
        import_identifier_queries: "[(import_statement)] @import_type".to_owned(),
        block_start: Some(":".to_owned()),
        variable_identifier_queries: vec!["(assignment left: (identifier) @identifier)".to_owned()],
        outline_query: Some(
            r#"
        (class_definition
            name: (identifier) @definition.class.name
        ) @definition.class
    
        (function_definition
            name: (identifier) @function.name
            parameters: (parameters
                (typed_parameter
                      (identifier) @parameters.identifier
                      type: (type) @parameter.type
                 )
                 (identifier) @parameters.identifier
            ) @function.parameters
            return_type: (type)? @function.return_type
            body: (block) @function.body
        ) @definition.function
    
        (assignment
            left: (identifier) @function.name
            type: (type) @function.parameters
            right: (lambda) @function.body
        ) @definition.function    
        "#
            .to_owned(),
        ),
        excluded_file_paths: vec![],
        language_str: "python".to_owned(),
    }
}
