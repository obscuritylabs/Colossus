use super::*;

/// Restricted non-executable workflow condition.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition(Expr);

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Exists(String),
    Compare(Operand, Compare, Operand),
    Not(Box<Self>),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
}

#[derive(Clone, Debug, PartialEq)]
enum Operand {
    Pointer(String),
    Literal(Value),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Compare {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Exists,
    Pointer(String),
    Literal(Value),
    LParen,
    RParen,
    Not,
    And,
    Or,
    Compare(Compare),
}

impl Condition {
    /// Parse and validate the entire restricted expression.
    pub fn parse(source: &str) -> Result<Self, WorkflowError> {
        if source.len() > MAX_CONDITION_BYTES {
            return Err(WorkflowError::InvalidDefinition(format!(
                "condition exceeds {MAX_CONDITION_BYTES} bytes"
            )));
        }
        let tokens = tokenize(source)?;
        if tokens.len() > MAX_CONDITION_TOKENS {
            return Err(WorkflowError::InvalidDefinition(format!(
                "condition exceeds {MAX_CONDITION_TOKENS} tokens"
            )));
        }
        let mut parser = Parser {
            tokens,
            position: 0,
            depth: 0,
            complexity: 0,
        };
        let expression = parser.parse_or()?;
        if parser.position != parser.tokens.len() {
            return Err(WorkflowError::InvalidDefinition(
                "condition has trailing tokens".into(),
            ));
        }
        Ok(Self(expression))
    }

    /// Evaluate against a bounded JSON workflow context.
    pub fn evaluate(&self, context: &Value) -> bool {
        evaluate_expr(&self.0, context)
    }
}

fn tokenize(source: &str) -> Result<Vec<Token>, WorkflowError> {
    let chars = source.as_bytes();
    let mut position = 0;
    let mut tokens = Vec::new();
    while position < chars.len() {
        if chars[position].is_ascii_whitespace() {
            position += 1;
            continue;
        }
        let tail = &source[position..];
        let (token, consumed) = if tail.starts_with("exists") {
            (Token::Exists, 6)
        } else if tail.starts_with("&&") {
            (Token::And, 2)
        } else if tail.starts_with("||") {
            (Token::Or, 2)
        } else if tail.starts_with("==") {
            (Token::Compare(Compare::Eq), 2)
        } else if tail.starts_with("!=") {
            (Token::Compare(Compare::Ne), 2)
        } else if tail.starts_with(">=") {
            (Token::Compare(Compare::Ge), 2)
        } else if tail.starts_with("<=") {
            (Token::Compare(Compare::Le), 2)
        } else if tail.starts_with('!') {
            (Token::Not, 1)
        } else if tail.starts_with('>') {
            (Token::Compare(Compare::Gt), 1)
        } else if tail.starts_with('<') {
            (Token::Compare(Compare::Lt), 1)
        } else if tail.starts_with('(') {
            (Token::LParen, 1)
        } else if tail.starts_with(')') {
            (Token::RParen, 1)
        } else if tail.starts_with('/') {
            let length = tail
                .find(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '(' | ')' | '!' | '=' | '<' | '>' | '&' | '|')
                })
                .unwrap_or(tail.len());
            (Token::Pointer(tail[..length].into()), length)
        } else {
            let length = if tail.starts_with('"') {
                json_string_length(tail)?
            } else {
                tail.find(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '(' | ')' | '!' | '=' | '<' | '>' | '&' | '|')
                })
                .unwrap_or(tail.len())
            };
            let literal = serde_json::from_str(&tail[..length]).map_err(|_| {
                WorkflowError::InvalidDefinition(format!(
                    "condition literal {:?} is not JSON",
                    &tail[..length]
                ))
            })?;
            (Token::Literal(literal), length)
        };
        tokens.push(token);
        position += consumed;
    }
    if tokens.is_empty() {
        return Err(WorkflowError::InvalidDefinition(
            "condition cannot be empty".into(),
        ));
    }
    Ok(tokens)
}

fn json_string_length(source: &str) -> Result<usize, WorkflowError> {
    let mut escaped = false;
    for (index, character) in source.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return Ok(index + 1);
        }
    }
    Err(WorkflowError::InvalidDefinition(
        "unterminated JSON string in condition".into(),
    ))
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
    depth: usize,
    complexity: usize,
}

impl Parser {
    fn add_boolean_node(&mut self) -> Result<(), WorkflowError> {
        if self.complexity >= MAX_CONDITION_DEPTH {
            return Err(WorkflowError::InvalidDefinition(format!(
                "condition boolean complexity exceeds {MAX_CONDITION_DEPTH} nodes"
            )));
        }
        self.complexity += 1;
        Ok(())
    }

    fn nested<T>(
        &mut self,
        parse: impl FnOnce(&mut Self) -> Result<T, WorkflowError>,
    ) -> Result<T, WorkflowError> {
        if self.depth >= MAX_CONDITION_DEPTH {
            return Err(WorkflowError::InvalidDefinition(format!(
                "condition nesting exceeds {MAX_CONDITION_DEPTH} levels"
            )));
        }
        self.depth += 1;
        let result = parse(self);
        self.depth -= 1;
        result
    }

    fn parse_or(&mut self) -> Result<Expr, WorkflowError> {
        let mut expression = self.parse_and()?;
        while self.consume(&Token::Or) {
            self.add_boolean_node()?;
            expression = Expr::Or(Box::new(expression), Box::new(self.parse_and()?));
        }
        Ok(expression)
    }

    fn parse_and(&mut self) -> Result<Expr, WorkflowError> {
        let mut expression = self.parse_unary()?;
        while self.consume(&Token::And) {
            self.add_boolean_node()?;
            expression = Expr::And(Box::new(expression), Box::new(self.parse_unary()?));
        }
        Ok(expression)
    }

    fn parse_unary(&mut self) -> Result<Expr, WorkflowError> {
        if self.consume(&Token::Not) {
            self.add_boolean_node()?;
            return self
                .nested(Self::parse_unary)
                .map(|expression| Expr::Not(Box::new(expression)));
        }
        if self.consume(&Token::LParen) {
            let expression = self.nested(Self::parse_or)?;
            if !self.consume(&Token::RParen) {
                return Err(WorkflowError::InvalidDefinition(
                    "condition is missing a closing parenthesis".into(),
                ));
            }
            return Ok(expression);
        }
        if self.consume(&Token::Exists) {
            if !self.consume(&Token::LParen) {
                return Err(WorkflowError::InvalidDefinition(
                    "exists requires parentheses".into(),
                ));
            }
            let pointer = match self.next() {
                Some(Token::Pointer(pointer)) => pointer,
                _ => {
                    return Err(WorkflowError::InvalidDefinition(
                        "exists requires a JSON pointer".into(),
                    ));
                }
            };
            if !self.consume(&Token::RParen) {
                return Err(WorkflowError::InvalidDefinition(
                    "exists is missing a closing parenthesis".into(),
                ));
            }
            return Ok(Expr::Exists(pointer));
        }
        let left = self.parse_operand()?;
        let comparison = match self.next() {
            Some(Token::Compare(comparison)) => comparison,
            _ => {
                return Err(WorkflowError::InvalidDefinition(
                    "conditions must use exists or an explicit comparison".into(),
                ));
            }
        };
        let right = self.parse_operand()?;
        Ok(Expr::Compare(left, comparison, right))
    }

    fn parse_operand(&mut self) -> Result<Operand, WorkflowError> {
        match self.next() {
            Some(Token::Pointer(pointer)) => Ok(Operand::Pointer(pointer)),
            Some(Token::Literal(value)) => Ok(Operand::Literal(value)),
            _ => Err(WorkflowError::InvalidDefinition(
                "condition comparison operand is missing".into(),
            )),
        }
    }

    fn consume(&mut self, expected: &Token) -> bool {
        if self.tokens.get(self.position) == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += usize::from(token.is_some());
        token
    }
}

fn operand_value(operand: &Operand, context: &Value) -> Option<Value> {
    match operand {
        Operand::Pointer(pointer) => context.pointer(pointer).cloned(),
        Operand::Literal(value) => Some(value.clone()),
    }
}

fn evaluate_expr(expression: &Expr, context: &Value) -> bool {
    match expression {
        Expr::Exists(pointer) => context.pointer(pointer).is_some(),
        Expr::Not(expression) => !evaluate_expr(expression, context),
        Expr::And(left, right) => evaluate_expr(left, context) && evaluate_expr(right, context),
        Expr::Or(left, right) => evaluate_expr(left, context) || evaluate_expr(right, context),
        Expr::Compare(left, comparison, right) => {
            let (Some(left), Some(right)) =
                (operand_value(left, context), operand_value(right, context))
            else {
                return false;
            };
            match comparison {
                Compare::Eq => left == right,
                Compare::Ne => left != right,
                Compare::Gt | Compare::Ge | Compare::Lt | Compare::Le => {
                    compare_order(&left, &right, *comparison)
                }
            }
        }
    }
}

fn compare_order(left: &Value, right: &Value, comparison: Compare) -> bool {
    let ordering = match (left, right) {
        (Value::Number(left), Value::Number(right)) => left
            .as_f64()
            .zip(right.as_f64())
            .and_then(|(left, right)| left.partial_cmp(&right)),
        (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
        _ => None,
    };
    ordering.is_some_and(|ordering| match comparison {
        Compare::Gt => ordering.is_gt(),
        Compare::Ge => ordering.is_ge(),
        Compare::Lt => ordering.is_lt(),
        Compare::Le => ordering.is_le(),
        Compare::Eq | Compare::Ne => false,
    })
}
