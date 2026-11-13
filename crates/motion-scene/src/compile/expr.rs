//! Arithmetic expression parsing and evaluation over scene-local time.

use motion_core::{Signal, Time};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Largest accepted expression source in bytes.
const MAX_EXPRESSION_BYTES: usize = 4096;

/// Largest accepted syntax tree size.
const MAX_NODES: usize = 1024;

/// Deepest accepted parenthesis, call and unary nesting.
const MAX_DEPTH: usize = 48;

/// A parsed expression and the distinct element references it reads.
pub(crate) struct Parsed {
    pub(crate) expr: Expr,
    pub(crate) references: Vec<String>,
}

pub(crate) enum Expr {
    Number(f64),
    Duration,
    Reference(usize),
    Negate(Box<Expr>),
    Binary(Operator, Box<Expr>, Box<Expr>),
    Call(Function, Vec<Expr>),
}

#[derive(Clone, Copy)]
pub(crate) enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Power,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    Equal,
    NotEqual,
}

#[derive(Clone, Copy)]
pub(crate) enum Function {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Sqrt,
    Abs,
    Floor,
    Ceil,
    Round,
    Fract,
    Exp,
    Ln,
    Sign,
    Radians,
    Degrees,
    Min,
    Max,
    Pow,
    Mod,
    Clamp,
    Lerp,
    Smoothstep,
    Step,
    If,
}

/// A compiled expression sampled as a numeric animation.
pub(crate) struct ExprAnimation {
    expr: Expr,
    references: Vec<Arc<dyn Signal<f64>>>,
}

impl ExprAnimation {
    pub(crate) fn new(expr: Expr, references: Vec<Arc<dyn Signal<f64>>>) -> Self {
        Self { expr, references }
    }
}

impl Signal<f64> for ExprAnimation {
    fn sample(&self, time: Time) -> f64 {
        let values: Vec<f64> = self
            .references
            .iter()
            .map(|reference| reference.sample(time))
            .collect();
        self.expr.eval(time.as_seconds(), &values)
    }
}

impl Expr {
    pub(super) fn eval(&self, time: f64, references: &[f64]) -> f64 {
        match self {
            Self::Number(value) => *value,
            Self::Duration => time,
            Self::Reference(index) => references[*index],
            Self::Negate(value) => -value.eval(time, references),
            Self::Binary(operator, left, right) => {
                operator.apply(left.eval(time, references), right.eval(time, references))
            }
            Self::Call(function, arguments) => {
                let mut values = [0.0; 3];
                for (slot, argument) in values.iter_mut().zip(arguments) {
                    *slot = argument.eval(time, references);
                }
                function.apply(values)
            }
        }
    }
}

impl Operator {
    fn apply(self, left: f64, right: f64) -> f64 {
        let flag = |value: bool| if value { 1.0 } else { 0.0 };
        match self {
            Self::Add => left + right,
            Self::Subtract => left - right,
            Self::Multiply => left * right,
            Self::Divide => left / right,
            Self::Remainder => left.rem_euclid(right),
            Self::Power => left.powf(right),
            Self::Less => flag(left < right),
            Self::Greater => flag(left > right),
            Self::LessEqual => flag(left <= right),
            Self::GreaterEqual => flag(left >= right),
            Self::Equal => flag(left == right),
            Self::NotEqual => flag(left != right),
        }
    }
}

impl Function {
    fn lookup(name: &str) -> Option<(Self, usize)> {
        Some(match name {
            "sin" => (Self::Sin, 1),
            "cos" => (Self::Cos, 1),
            "tan" => (Self::Tan, 1),
            "asin" => (Self::Asin, 1),
            "acos" => (Self::Acos, 1),
            "atan" => (Self::Atan, 1),
            "atan2" => (Self::Atan2, 2),
            "sqrt" => (Self::Sqrt, 1),
            "abs" => (Self::Abs, 1),
            "floor" => (Self::Floor, 1),
            "ceil" => (Self::Ceil, 1),
            "round" => (Self::Round, 1),
            "fract" => (Self::Fract, 1),
            "exp" => (Self::Exp, 1),
            "ln" => (Self::Ln, 1),
            "sign" => (Self::Sign, 1),
            "radians" => (Self::Radians, 1),
            "degrees" => (Self::Degrees, 1),
            "min" => (Self::Min, 2),
            "max" => (Self::Max, 2),
            "pow" => (Self::Pow, 2),
            "mod" => (Self::Mod, 2),
            "clamp" => (Self::Clamp, 3),
            "lerp" => (Self::Lerp, 3),
            "smoothstep" => (Self::Smoothstep, 3),
            "step" => (Self::Step, 2),
            "if" => (Self::If, 3),
            _ => return None,
        })
    }

    fn apply(self, [a, b, c]: [f64; 3]) -> f64 {
        match self {
            Self::Sin => a.sin(),
            Self::Cos => a.cos(),
            Self::Tan => a.tan(),
            Self::Asin => a.asin(),
            Self::Acos => a.acos(),
            Self::Atan => a.atan(),
            Self::Atan2 => a.atan2(b),
            Self::Sqrt => a.sqrt(),
            Self::Abs => a.abs(),
            Self::Floor => a.floor(),
            Self::Ceil => a.ceil(),
            Self::Round => a.round(),
            Self::Fract => a - a.floor(),
            Self::Exp => a.exp(),
            Self::Ln => a.ln(),
            Self::Sign => {
                if a == 0.0 {
                    0.0
                } else {
                    a.signum()
                }
            }
            Self::Radians => a.to_radians(),
            Self::Degrees => a.to_degrees(),
            Self::Min => a.min(b),
            Self::Max => a.max(b),
            Self::Pow => a.powf(b),
            Self::Mod => a.rem_euclid(b),
            Self::Clamp => a.max(b).min(c),
            Self::Lerp => a + (b - a) * c,
            Self::Smoothstep => {
                let progress = ((c - a) / (b - a)).clamp(0.0, 1.0);
                progress * progress * (3.0 - 2.0 * progress)
            }
            Self::Step => {
                if b >= a {
                    1.0
                } else {
                    0.0
                }
            }
            Self::If => {
                if a != 0.0 {
                    b
                } else {
                    c
                }
            }
        }
    }
}

/// Every function name, for suggestions.
const FUNCTION_NAMES: [&str; 27] = [
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sqrt",
    "abs",
    "floor",
    "ceil",
    "round",
    "fract",
    "exp",
    "ln",
    "sign",
    "radians",
    "degrees",
    "min",
    "max",
    "pow",
    "mod",
    "clamp",
    "lerp",
    "smoothstep",
    "step",
    "if",
];

fn with_hint<'a>(
    mut message: String,
    name: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> String {
    if let Some(hint) = crate::diagnostics::did_you_mean(name, candidates) {
        message.push_str(". ");
        message.push_str(&hint);
    }
    message
}

/// Parses `source`, resolving `$name` against the numeric `parameters`.
///
/// Grammar, from loosest to tightest: comparison, `+ -`, `* / %`, unary `-`,
/// right-associative `^`, then numbers, `t` (scene-local seconds), `pi`, `tau`,
/// `$parameter`, `@element.property` references, function calls and parentheses.
pub(crate) fn parse(source: &str, parameters: &BTreeMap<String, Value>) -> Result<Parsed, String> {
    parse_with(
        source,
        &mut |name| {
            let value = match parameters.get(name) {
                Some(Value::Number(number)) => number.as_f64(),
                Some(Value::Bool(flag)) => Some(f64::from(u8::from(*flag))),
                Some(_) => return Err(format!("Parameter `{name}` is not a number")),
                None => {
                    return Err(with_hint(
                        format!("Unknown parameter `{name}`"),
                        name,
                        parameters.keys().map(String::as_str),
                    ));
                }
            };
            value.ok_or_else(|| format!("Parameter `{name}` is not finite"))
        },
        true,
    )
}

/// Evaluates a finite load-time number without scene time or element bindings.
pub(crate) fn calculate(
    source: &str,
    parameters: &mut dyn FnMut(&str) -> Result<f64, String>,
) -> Result<f64, String> {
    let parsed = parse_with(source, parameters, false)?;
    let value = parsed.expr.eval(0.0, &[]);
    if value.is_finite() {
        Ok(value)
    } else {
        Err("Load-time calculations must produce a finite number".into())
    }
}

fn parse_with(
    source: &str,
    parameters: &mut dyn FnMut(&str) -> Result<f64, String>,
    animated: bool,
) -> Result<Parsed, String> {
    if source.len() > MAX_EXPRESSION_BYTES {
        return Err("Expression exceeds 4096 bytes".into());
    }
    let mut parser = Parser {
        bytes: source.as_bytes(),
        position: 0,
        nodes: 0,
        depth: 0,
        parameters,
        animated,
        references: Vec::new(),
    };
    let expr = parser.comparison()?;
    parser.skip_space();
    if parser.position < parser.bytes.len() {
        return Err(parser.unexpected());
    }
    Ok(Parsed {
        expr,
        references: parser.references,
    })
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
    nodes: usize,
    depth: usize,
    parameters: &'a mut dyn FnMut(&str) -> Result<f64, String>,
    animated: bool,
    references: Vec<String>,
}

impl Parser<'_> {
    fn comparison(&mut self) -> Result<Expr, String> {
        let left = self.sum()?;
        self.skip_space();
        let operator = match (self.peek(), self.bytes.get(self.position + 1).copied()) {
            (Some(b'<'), Some(b'=')) => Some((Operator::LessEqual, 2)),
            (Some(b'>'), Some(b'=')) => Some((Operator::GreaterEqual, 2)),
            (Some(b'='), Some(b'=')) => Some((Operator::Equal, 2)),
            (Some(b'!'), Some(b'=')) => Some((Operator::NotEqual, 2)),
            (Some(b'<'), _) => Some((Operator::Less, 1)),
            (Some(b'>'), _) => Some((Operator::Greater, 1)),
            _ => None,
        };
        let Some((operator, length)) = operator else {
            return Ok(left);
        };
        self.position += length;
        let right = self.sum()?;
        self.node(Expr::Binary(operator, Box::new(left), Box::new(right)))
    }

    fn sum(&mut self) -> Result<Expr, String> {
        let mut left = self.product()?;
        loop {
            self.skip_space();
            let operator = match self.peek() {
                Some(b'+') => Operator::Add,
                Some(b'-') => Operator::Subtract,
                _ => return Ok(left),
            };
            self.position += 1;
            let right = self.product()?;
            left = self.node(Expr::Binary(operator, Box::new(left), Box::new(right)))?;
        }
    }

    fn product(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        loop {
            self.skip_space();
            let operator = match self.peek() {
                Some(b'*') => Operator::Multiply,
                Some(b'/') => Operator::Divide,
                Some(b'%') => Operator::Remainder,
                _ => return Ok(left),
            };
            self.position += 1;
            let right = self.unary()?;
            left = self.node(Expr::Binary(operator, Box::new(left), Box::new(right)))?;
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        self.skip_space();
        match self.peek() {
            Some(b'-') => {
                self.position += 1;
                self.enter()?;
                let value = self.unary()?;
                self.depth -= 1;
                self.node(Expr::Negate(Box::new(value)))
            }
            Some(b'+') => {
                self.position += 1;
                self.enter()?;
                let value = self.unary()?;
                self.depth -= 1;
                Ok(value)
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Expr, String> {
        let base = self.atom()?;
        self.skip_space();
        if self.peek() != Some(b'^') {
            return Ok(base);
        }
        self.position += 1;
        self.enter()?;
        let exponent = self.unary()?;
        self.depth -= 1;
        self.node(Expr::Binary(
            Operator::Power,
            Box::new(base),
            Box::new(exponent),
        ))
    }

    fn atom(&mut self) -> Result<Expr, String> {
        self.skip_space();
        match self.peek() {
            Some(b'(') => {
                self.position += 1;
                self.enter()?;
                let value = self.comparison()?;
                self.depth -= 1;
                self.consume(b')')?;
                Ok(value)
            }
            Some(b'0'..=b'9' | b'.') => self.number(),
            Some(b'$') => self.parameter(),
            Some(b'@') => self.reference(),
            Some(byte) if byte.is_ascii_alphabetic() || byte == b'_' => self.name(),
            _ => Err(self.unexpected()),
        }
    }

    fn number(&mut self) -> Result<Expr, String> {
        let start = self.position;
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_digit() || byte == b'.')
        {
            self.position += 1;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let sign = usize::from(matches!(
                self.bytes.get(self.position + 1),
                Some(b'+' | b'-')
            ));
            if self
                .bytes
                .get(self.position + 1 + sign)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1 + sign;
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.position += 1;
                }
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.position]).unwrap_or_default();
        let value = text
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("Invalid number `{text}` at column {}", start + 1))?;
        self.node(Expr::Number(value))
    }

    fn parameter(&mut self) -> Result<Expr, String> {
        self.position += 1;
        let name = self.word(|byte| byte.is_ascii_alphanumeric() || byte == b'_')?;
        let value = (self.parameters)(&name)?;
        self.node(Expr::Number(value))
    }

    fn reference(&mut self) -> Result<Expr, String> {
        if !self.animated {
            return Err("Load-time calculations cannot reference elements".into());
        }
        self.position += 1;
        let name =
            self.word(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.')?;
        let index = match self.references.iter().position(|known| *known == name) {
            Some(index) => index,
            None => {
                self.references.push(name);
                self.references.len() - 1
            }
        };
        self.node(Expr::Reference(index))
    }

    fn name(&mut self) -> Result<Expr, String> {
        let name = self.word(|byte| byte.is_ascii_alphanumeric() || byte == b'_')?;
        self.skip_space();
        if self.peek() != Some(b'(') {
            return match name.as_str() {
                "t" if self.animated => self.node(Expr::Duration),
                "t" => Err("Load-time calculations cannot read time `t`".into()),
                "pi" => self.node(Expr::Number(std::f64::consts::PI)),
                "tau" => self.node(Expr::Number(std::f64::consts::TAU)),
                _ => Err(with_hint(
                    format!("Unknown name `{name}`"),
                    &name,
                    ["t", "pi", "tau"],
                )),
            };
        }
        let (function, arity) = Function::lookup(&name).ok_or_else(|| {
            with_hint(
                format!("Unknown function `{name}`"),
                &name,
                FUNCTION_NAMES.iter().copied(),
            )
        })?;
        self.position += 1;
        self.enter()?;
        let mut arguments = Vec::with_capacity(arity);
        loop {
            arguments.push(self.comparison()?);
            self.skip_space();
            if self.peek() == Some(b',') {
                self.position += 1;
            } else {
                break;
            }
        }
        self.depth -= 1;
        self.consume(b')')?;
        if arguments.len() != arity {
            return Err(format!(
                "`{name}` takes {arity} argument(s), got {}",
                arguments.len()
            ));
        }
        self.node(Expr::Call(function, arguments))
    }

    fn word(&mut self, accept: impl Fn(u8) -> bool) -> Result<String, String> {
        let start = self.position;
        while self.peek().is_some_and(&accept) {
            self.position += 1;
        }
        if start == self.position {
            return Err(self.unexpected());
        }
        Ok(String::from_utf8_lossy(&self.bytes[start..self.position]).into_owned())
    }

    fn consume(&mut self, byte: u8) -> Result<(), String> {
        self.skip_space();
        if self.peek() == Some(byte) {
            self.position += 1;
            Ok(())
        } else {
            Err(format!(
                "Expected `{}` at column {}",
                char::from(byte),
                self.position + 1
            ))
        }
    }

    fn node(&mut self, expr: Expr) -> Result<Expr, String> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err("Expression exceeds 1024 terms".into());
        }
        Ok(expr)
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err("Expression nests deeper than 48 levels".into());
        }
        Ok(())
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn unexpected(&self) -> String {
        match self.peek() {
            Some(byte) => format!(
                "Unexpected `{}` at column {}",
                char::from(byte),
                self.position + 1
            ),
            None => "Unexpected end of expression".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculations_resolve_parameters_and_reject_runtime_inputs() {
        let mut parameter = |name: &str| match name {
            "seconds" => Ok(8.0),
            _ => Err(format!("Unknown parameter `{name}`")),
        };
        assert_eq!(
            calculate("ceil($seconds * 30 / 43) / 30", &mut parameter).unwrap(),
            6.0 / 30.0
        );
        for expression in ["t", "@dot.radius", "1 / 0", "sqrt(-1)", "$missing"] {
            assert!(
                calculate(expression, &mut parameter).is_err(),
                "{expression}"
            );
        }
    }

    fn eval(source: &str, time: f64) -> Result<f64, String> {
        let mut parameters = BTreeMap::new();
        parameters.insert("size".into(), serde_json::json!(10));
        let parsed = parse(source, &parameters)?;
        Ok(parsed.expr.eval(time, &[]))
    }

    #[test]
    fn precedence_associativity_and_functions() {
        assert_eq!(eval("1 + 2 * 3", 0.0), Ok(7.0));
        assert_eq!(eval("-2 ^ 2", 0.0), Ok(-4.0));
        assert_eq!(eval("2 ^ 3 ^ 2", 0.0), Ok(512.0));
        assert_eq!(eval("(1 + 2) * $size - t", 4.0), Ok(26.0));
        assert_eq!(eval("clamp(t * 3, 0, 1) + min(1, 2)", 1.0), Ok(2.0));
        assert_eq!(eval("if(t >= 2, 5, 6)", 2.0), Ok(5.0));
        assert_eq!(eval("-1 % 3 + 1e1 + .5", 0.0), Ok(12.5));
        assert_eq!(eval("lerp(0, 10, smoothstep(0, 2, t))", 1.0), Ok(5.0));
    }

    #[test]
    fn invalid_sources_report_positions() {
        for source in [
            "1 +",
            "(1",
            "foo(1)",
            "sin(1, 2)",
            "$nope",
            "bar",
            "1 2",
            "@",
        ] {
            assert!(eval(source, 0.0).is_err(), "{source}");
        }
        assert!(eval("1 +", 0.0).unwrap_err().contains("end of expression"));
        let deep = format!("{}1{}", "(".repeat(100), ")".repeat(100));
        assert!(eval(&deep, 0.0).unwrap_err().contains("48"));
        assert!(eval(&"1+".repeat(2000), 0.0).is_err());
    }

    #[test]
    fn references_are_deduplicated_in_order() {
        let parsed = parse("@a.x + @b.y * @a.x", &BTreeMap::new()).unwrap();
        assert_eq!(parsed.references, ["a.x", "b.y"]);
        assert_eq!(parsed.expr.eval(0.0, &[2.0, 3.0]), 8.0);
    }
}
