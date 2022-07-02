use super::*;
use crate::ast::{Module, Visibility};
use crate::error::SyntaxError;
use crate::span::FileId;

impl Parser {
    pub fn parse_all_top_level(&mut self, file_id: FileId) -> ParserResult {
        let mut module = ast::Module::new(file_id, self.module_info);

        while !self.is_end() {
            match self.parse_top_level(&mut module) {
                Ok(_) => {
                    if let Err(_) = require!(self, Semicolon, ";") {
                        let span = Parser::get_missing_delimiter_span(self.previous_span());
                        self.cache
                            .lock()
                            .diagnostics
                            .push(SyntaxError::expected(span, ";"));
                        self.skip_until_recovery_point();
                    }
                }
                Err(diag) => {
                    self.cache.lock().diagnostics.push(diag);
                    self.skip_until_recovery_point();
                }
            }

            self.skip_semicolons();
        }

        ParserResult::NewAst(module)
    }

    pub fn parse_top_level(&mut self, module: &mut Module) -> DiagnosticResult<()> {
        let visibility = if eat!(self, Pub) {
            Visibility::Public
        } else {
            Visibility::Private
        };

        if eat!(self, Let) {
            let start_span = self.previous_span();

            let binding = if eat!(self, Extern) {
                self.parse_extern(visibility, start_span)?
            } else if eat!(self, Intrinsic) {
                self.parse_builtin_binding(visibility, start_span)?
            } else {
                self.parse_binding(visibility)?
            };

            module.bindings.push(binding);

            Ok(())
        } else if eat!(self, Ident(_)) {
            let token = self.previous().clone();
            let symbol = token.name();

            require!(self, Bang, "!")?;
            require!(self, OpenParen, "(")?;

            if symbol == "run" {
                let expr = self.parse_expr()?;
                module.run_exprs.push(expr);
                require!(self, CloseParen, ")")?;

                Ok(())
            } else {
                Err(SyntaxError::expected(self.previous_span(), "run"))
            }
        } else {
            Err(SyntaxError::expected(
                self.span(),
                &format!("an item, got `{}`", self.peek().lexeme),
            ))
        }
    }
}
