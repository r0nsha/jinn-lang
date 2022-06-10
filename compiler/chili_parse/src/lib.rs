mod binding;
mod expr;
mod r#extern;
mod func;
mod import;
mod literal;
mod pattern;
mod postfix_expr;
mod top_level;

use bitflags::bitflags;
use chili_ast::{
    ast::{self, ExternLibrary},
    workspace::ModuleInfo,
};
use chili_error::{diagnostic::Diagnostic, DiagnosticResult, Diagnostics, SyntaxError};
use chili_span::{EndPosition, Position, Span};
use chili_token::{lexer::Lexer, Token, TokenKind::*};
use parking_lot::Mutex;
use std::{
    collections::HashSet,
    fmt::Debug,
    path::PathBuf,
    sync::{mpsc::Sender, Arc},
};
use threadpool::ThreadPool;
use unindent::unindent;
use ustr::{ustr, Ustr};

bitflags! {
    struct Restrictions : u8 {
        const STMT_EXPR = 1 << 0;
        const NO_STRUCT_LITERAL = 1 << 1;
    }
}

macro_rules! is {
    ($parser:expr, $(|) ? $($pattern : pat_param) | +) => {
        if $parser.is_end() {
            false
        } else {
            match &$parser.peek().kind {
                $( $pattern )|+ => true,
                _ => false
            }
        }
    };
}

macro_rules! eat {
    ($parser:expr, $(|) ? $($pattern : pat_param) | +) => {
        if is!($parser, $( $pattern )|+) {
            $parser.bump();
            true
        } else {
            false
        }
    };
}

macro_rules! require {
    ($parser:expr, $(|) ? $($pattern : pat_param) | +, $msg:expr) => {
        if is!($parser, $( $pattern )|+) {
            Ok($parser.bump().clone())
        } else {
            Err(SyntaxError::expected($parser.span(), $msg))
        }
    };
}

macro_rules! parse_delimited_list {
    ($parser:expr, $close_delim:pat, $sep:pat, $parse:expr, $msg:expr) => {{
        let mut items = vec![];

        while !eat!($parser, $close_delim) && !$parser.is_end() {
            items.push($parse);

            if eat!($parser, $sep) {
                continue;
            } else if eat!($parser, $close_delim) {
                break;
            } else {
                let span = Parser::get_missing_delimiter_span($parser.previous_span());
                return Err(SyntaxError::expected(span, $msg));
            }
        }

        items
    }};
}

pub(crate) use eat;
pub(crate) use is;
pub(crate) use parse_delimited_list;
pub(crate) use require;

pub fn spawn_parser(
    thread_pool: ThreadPool,
    tx: Sender<Box<ParserResult>>,
    cache: Arc<Mutex<ParserCache>>,
    module_info: ModuleInfo,
) {
    thread_pool.clone().execute(move || {
        Parser::new(thread_pool, tx, Arc::clone(&cache), module_info).parse();
    });
}

pub struct Parser {
    pub cache: Arc<Mutex<ParserCache>>,
    thread_pool: ThreadPool,
    tx: Sender<Box<ParserResult>>,
    tokens: Vec<Token>,
    current: usize,
    module_info: ModuleInfo,
    current_dir: PathBuf,
    decl_name_frames: Vec<Ustr>,
    restrictions: Restrictions,
    extern_lib: Option<Option<ExternLibrary>>,
}

#[derive(Debug)]
pub struct ParserCache {
    pub root_file: PathBuf,
    pub root_dir: PathBuf,
    pub std_dir: PathBuf,
    pub diagnostics: Diagnostics,
    pub parsed_modules: HashSet<ModuleInfo>,
    pub total_lines: u32,
}

pub enum ParserResult {
    NewAst(ast::Ast),
    AlreadyParsed,
    Failed(Diagnostic),
}

impl Parser {
    pub fn new(
        thread_pool: ThreadPool,
        tx: Sender<Box<ParserResult>>,
        cache: Arc<Mutex<ParserCache>>,
        module_info: ModuleInfo,
    ) -> Self {
        Self {
            cache,
            thread_pool,
            tx,
            tokens: vec![],
            current: 0,
            module_info,
            current_dir: module_info.dir().to_path_buf(),
            decl_name_frames: Default::default(),
            restrictions: Restrictions::empty(),
            extern_lib: None,
        }
    }

    pub fn parse(mut self) {
        let result = self.parse_inner();
        self.tx.send(Box::new(result)).unwrap();
    }

    fn parse_inner(&mut self) -> ParserResult {
        let (file_id, source) = {
            let mut cache = self.cache.lock();

            if !cache.parsed_modules.insert(self.module_info) {
                return ParserResult::AlreadyParsed;
            } else {
                let source = std::fs::read_to_string(self.module_info.file_path.as_str())
                    .unwrap_or_else(|_| panic!("failed to read `{}`", self.module_info.file_path));

                cache.total_lines += source.lines().count() as u32;

                let file_id = cache
                    .diagnostics
                    .add_file(self.module_info.file_path.to_string(), unindent(&source));

                (file_id, source)
            }
        };

        match Lexer::new(file_id, &source).scan() {
            Ok(tokens) => {
                self.tokens = tokens;

                let mut ast = ast::Ast::new(file_id, self.module_info);

                while !self.is_end() {
                    if let Err(diag) = self.parse_top_level(&mut ast) {
                        return ParserResult::Failed(diag);
                    }

                    if let Err(_) = require!(self, Semicolon, ";") {
                        let span = Parser::get_missing_delimiter_span(self.previous_span());
                        return ParserResult::Failed(SyntaxError::expected(span, ";"));
                    }

                    self.skip_semicolons();
                }

                ParserResult::NewAst(ast)
            }
            Err(diag) => ParserResult::Failed(diag),
        }
    }

    pub(crate) fn with_res<T>(
        &mut self,
        restrictions: Restrictions,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let old = self.restrictions;
        self.restrictions = restrictions;
        let res = f(self);
        self.restrictions = old;
        res
    }

    pub(crate) fn get_decl_name(&self) -> Ustr {
        if !self.decl_name_frames.is_empty() {
            *self.decl_name_frames.last().unwrap()
        } else {
            ustr("")
        }
    }

    #[inline]
    pub(crate) fn bump(&mut self) -> &Token {
        if !self.is_end() {
            self.current += 1;
        }
        self.previous()
    }

    #[inline]
    pub(crate) fn is_end(&self) -> bool {
        self.peek().kind == Eof
    }

    #[inline]
    pub(crate) fn peek(&self) -> &Token {
        self.tokens.get(self.current).unwrap()
    }

    #[inline]
    pub(crate) fn previous(&self) -> &Token {
        self.tokens.get(self.current - 1).unwrap()
    }

    #[inline]
    pub(crate) fn span(&self) -> Span {
        self.peek().span
    }

    #[inline]
    pub(crate) fn previous_span(&self) -> Span {
        self.previous().span
    }

    #[inline]
    pub(crate) fn skip_semicolons(&mut self) {
        while is!(self, Semicolon) {
            self.bump();
        }
    }

    #[inline]
    pub(crate) fn get_missing_delimiter_span(after_span: Span) -> Span {
        let start_pos = Position {
            index: after_span.end.index,
            line: after_span.start.line,
            column: after_span.start.column,
        };

        let end_pos = EndPosition {
            index: after_span.end.index,
        };

        after_span.with_start(start_pos).with_end(end_pos)
    }
}
