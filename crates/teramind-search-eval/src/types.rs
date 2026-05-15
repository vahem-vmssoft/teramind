//! Query/qrels/report types. See Section 2.

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum QueryClass {
    NaturalLanguage,
    StackTrace,
    CodeSnippet,
    ToolTyped,
    SymbolicPath,
}
