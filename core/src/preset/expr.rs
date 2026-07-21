//! A tiny pure expression language over the audio-analysis variables, compiled
//! once at preset load and evaluated per parameter per frame.
//!
//! Grammar (recursive descent, standard precedence):
//!
//! ```text
//! expr   := term  (('+' | '-') term)*
//! term   := unary (('*' | '/') unary)*
//! unary  := ('-' | '+')? primary
//! primary:= number | ident | ident '(' expr (',' expr)* ')' | '(' expr ')'
//! ```
//!
//! Variables: `bass mid treb onset beat bar time`. Functions: `sin abs floor
//! min max clamp lerp`. Compilation is fallible (a malformed expression is
//! rejected with a surfaced error, never a panic); evaluation of a compiled
//! expression is total, panic-free, and allocation-free — it walks a prebuilt
//! AST returning `f32`, so it is safe to call every frame (hot-path §5).

// Hot-path panic-denial pragma: `eval` runs per parameter per frame. Preset/
// is not yet in the hygiene guard's scan (an architect/Mode-4 call); armed
// here so the per-frame evaluator is panic-denied regardless.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use std::fmt;

/// The analysis variables an expression may reference, in slot order.
pub const VAR_NAMES: [&str; 7] = ["bass", "mid", "treb", "onset", "beat", "bar", "time"];
/// Number of expression variables.
pub const VAR_COUNT: usize = VAR_NAMES.len();

/// A bound set of variable values for one evaluation. Field order matches
/// [`VAR_NAMES`]; `beat` is the caller's bool coerced to 0.0/1.0.
#[derive(Debug, Clone, Copy, Default)]
pub struct Variables {
    values: [f32; VAR_COUNT],
}

impl Variables {
    /// Bind all seven variables (order matches [`VAR_NAMES`]).
    #[allow(clippy::too_many_arguments)]
    pub fn new(bass: f32, mid: f32, treb: f32, onset: f32, beat: f32, bar: f32, time: f32) -> Self {
        Self {
            values: [bass, mid, treb, onset, beat, bar, time],
        }
    }

    /// Value in `slot` (0.0 for an out-of-range slot — never panics; compiled
    /// expressions only ever produce valid slots).
    fn get(&self, slot: usize) -> f32 {
        self.values.get(slot).copied().unwrap_or(0.0)
    }
}

/// Built-in functions, tagged with their arity so the parser can check it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Func {
    Sin,
    Abs,
    Floor,
    Min,
    Max,
    Clamp,
    Lerp,
}

impl Func {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "sin" => Func::Sin,
            "abs" => Func::Abs,
            "floor" => Func::Floor,
            "min" => Func::Min,
            "max" => Func::Max,
            "clamp" => Func::Clamp,
            "lerp" => Func::Lerp,
            _ => return None,
        })
    }

    fn arity(self) -> usize {
        match self {
            Func::Sin | Func::Abs | Func::Floor => 1,
            Func::Min | Func::Max => 2,
            Func::Clamp | Func::Lerp => 3,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Compiled AST node. `Box`/`Box<[_]>` allocate once at compile; evaluation
/// only reads them.
#[derive(Debug)]
enum Node {
    Const(f32),
    Var(usize),
    Neg(Box<Node>),
    Bin(BinOp, Box<Node>, Box<Node>),
    Call(Func, Box<[Node]>),
}

impl Node {
    fn eval(&self, vars: &Variables) -> f32 {
        match self {
            Node::Const(c) => *c,
            Node::Var(slot) => vars.get(*slot),
            Node::Neg(inner) => -inner.eval(vars),
            Node::Bin(op, l, r) => {
                let a = l.eval(vars);
                let b = r.eval(vars);
                match op {
                    BinOp::Add => a + b,
                    BinOp::Sub => a - b,
                    BinOp::Mul => a * b,
                    // f32 division by zero yields inf/NaN, not a panic — fine
                    // for a display value; expressions never divide silently.
                    BinOp::Div => a / b,
                }
            }
            // Arity is guaranteed by the parser; slice patterns keep this
            // indexing- and panic-free, with a safe default for completeness.
            Node::Call(func, args) => match (func, args.as_ref()) {
                (Func::Sin, [x]) => x.eval(vars).sin(),
                (Func::Abs, [x]) => x.eval(vars).abs(),
                (Func::Floor, [x]) => x.eval(vars).floor(),
                (Func::Min, [a, b]) => a.eval(vars).min(b.eval(vars)),
                (Func::Max, [a, b]) => a.eval(vars).max(b.eval(vars)),
                // Manual clamp: std f32::clamp panics if lo > hi; max().min()
                // is total.
                (Func::Clamp, [x, lo, hi]) => x.eval(vars).max(lo.eval(vars)).min(hi.eval(vars)),
                (Func::Lerp, [a, b, t]) => {
                    let a = a.eval(vars);
                    let b = b.eval(vars);
                    a + (b - a) * t.eval(vars)
                }
                _ => 0.0,
            },
        }
    }
}

/// A compiled expression: parse once, [`eval`](Expr::eval) every frame.
#[derive(Debug)]
pub struct Expr {
    root: Node,
}

impl Expr {
    /// Evaluate against a variable binding. Total and allocation-free.
    pub fn eval(&self, vars: &Variables) -> f32 {
        self.root.eval(vars)
    }
}

/// Why an expression failed to compile. Evaluation never errors.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprError {
    /// A character the tokenizer does not recognize.
    UnexpectedChar(char),
    /// A numeric literal that does not parse as `f32`.
    BadNumber(String),
    /// An identifier that is neither a known variable nor function.
    UnknownIdent(String),
    /// A function called with the wrong number of arguments.
    WrongArity {
        /// Function name.
        func: String,
        /// Arity the function requires.
        expected: usize,
        /// Arity supplied.
        got: usize,
    },
    /// A token appeared where the grammar did not allow it.
    UnexpectedToken(String),
    /// The expression ended earlier than the grammar allows.
    UnexpectedEnd,
    /// Extra tokens remained after a complete expression.
    TrailingTokens,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExprError::UnexpectedChar(c) => write!(f, "unexpected character '{c}'"),
            ExprError::BadNumber(s) => write!(f, "invalid number '{s}'"),
            ExprError::UnknownIdent(s) => write!(f, "unknown variable or function '{s}'"),
            ExprError::WrongArity {
                func,
                expected,
                got,
            } => write!(f, "{func}() takes {expected} argument(s), got {got}"),
            ExprError::UnexpectedToken(s) => write!(f, "unexpected token '{s}'"),
            ExprError::UnexpectedEnd => write!(f, "unexpected end of expression"),
            ExprError::TrailingTokens => write!(f, "unexpected trailing tokens"),
        }
    }
}

impl std::error::Error for ExprError {}

/// Compile a source expression into an evaluatable [`Expr`].
pub fn compile(src: &str) -> Result<Expr, ExprError> {
    let tokens = tokenize(src)?;
    let mut parser = Parser { tokens, pos: 0 };
    let root = parser.parse_expr()?;
    if parser.pos != parser.tokens.len() {
        return Err(ExprError::TrailingTokens);
    }
    Ok(Expr { root })
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f32),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Comma,
}

impl Token {
    fn describe(&self) -> String {
        match self {
            Token::Num(n) => n.to_string(),
            Token::Ident(s) => s.clone(),
            Token::Plus => "+".into(),
            Token::Minus => "-".into(),
            Token::Star => "*".into(),
            Token::Slash => "/".into(),
            Token::LParen => "(".into(),
            Token::RParen => ")".into(),
            Token::Comma => ",".into(),
        }
    }
}

fn tokenize(src: &str) -> Result<Vec<Token>, ExprError> {
    let mut tokens = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '+' => {
                chars.next();
                tokens.push(Token::Plus);
            }
            '-' => {
                chars.next();
                tokens.push(Token::Minus);
            }
            '*' => {
                chars.next();
                tokens.push(Token::Star);
            }
            '/' => {
                chars.next();
                tokens.push(Token::Slash);
            }
            '(' => {
                chars.next();
                tokens.push(Token::LParen);
            }
            ')' => {
                chars.next();
                tokens.push(Token::RParen);
            }
            ',' => {
                chars.next();
                tokens.push(Token::Comma);
            }
            c if c.is_ascii_digit() || c == '.' => {
                let mut num = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() || d == '.' {
                        num.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let value: f32 = num.parse().map_err(|_| ExprError::BadNumber(num.clone()))?;
                tokens.push(Token::Num(value));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut ident = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_alphanumeric() || d == '_' {
                        ident.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Ident(ident));
            }
            other => return Err(ExprError::UnexpectedChar(other)),
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&Token> {
        let tok = self.tokens.get(self.pos);
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn parse_expr(&mut self) -> Result<Node, ExprError> {
        let mut left = self.parse_term()?;
        while let Some(op) = match self.peek() {
            Some(Token::Plus) => Some(BinOp::Add),
            Some(Token::Minus) => Some(BinOp::Sub),
            _ => None,
        } {
            self.pos += 1;
            let right = self.parse_term()?;
            left = Node::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Node, ExprError> {
        let mut left = self.parse_unary()?;
        while let Some(op) = match self.peek() {
            Some(Token::Star) => Some(BinOp::Mul),
            Some(Token::Slash) => Some(BinOp::Div),
            _ => None,
        } {
            self.pos += 1;
            let right = self.parse_unary()?;
            left = Node::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Node, ExprError> {
        match self.peek() {
            Some(Token::Minus) => {
                self.pos += 1;
                Ok(Node::Neg(Box::new(self.parse_unary()?)))
            }
            Some(Token::Plus) => {
                self.pos += 1;
                self.parse_unary()
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Node, ExprError> {
        match self.advance() {
            Some(Token::Num(n)) => Ok(Node::Const(*n)),
            Some(Token::LParen) => {
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            Some(Token::Ident(name)) => {
                let name = name.clone();
                if matches!(self.peek(), Some(Token::LParen)) {
                    self.parse_call(name)
                } else if let Some(slot) = VAR_NAMES.iter().position(|&v| v == name) {
                    Ok(Node::Var(slot))
                } else {
                    Err(ExprError::UnknownIdent(name))
                }
            }
            Some(other) => Err(ExprError::UnexpectedToken(other.describe())),
            None => Err(ExprError::UnexpectedEnd),
        }
    }

    fn parse_call(&mut self, name: String) -> Result<Node, ExprError> {
        let func = Func::from_name(&name).ok_or(ExprError::UnknownIdent(name.clone()))?;
        self.expect(&Token::LParen)?;
        let mut args = Vec::new();
        if !matches!(self.peek(), Some(Token::RParen)) {
            loop {
                args.push(self.parse_expr()?);
                match self.peek() {
                    Some(Token::Comma) => {
                        self.pos += 1;
                    }
                    _ => break,
                }
            }
        }
        self.expect(&Token::RParen)?;
        if args.len() != func.arity() {
            return Err(ExprError::WrongArity {
                func: name,
                expected: func.arity(),
                got: args.len(),
            });
        }
        Ok(Node::Call(func, args.into_boxed_slice()))
    }

    fn expect(&mut self, want: &Token) -> Result<(), ExprError> {
        match self.advance() {
            Some(tok) if tok == want => Ok(()),
            Some(other) => Err(ExprError::UnexpectedToken(other.describe())),
            None => Err(ExprError::UnexpectedEnd),
        }
    }
}
