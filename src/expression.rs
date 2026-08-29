use crate::function::SurfaceFunction;
use crate::math;

pub const MAX_EXPRESSION_LENGTH: usize = 96;
pub const MAX_BYTECODE_LENGTH: usize = 64;
pub const MAX_OPERATOR_DEPTH: usize = 32;
pub const MAX_EVALUATION_DEPTH: usize = 32;

#[derive(Clone, Copy)]
enum Instruction {
    Constant(f32),
    X,
    Y,
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
    Negate,
    Sin,
    Cos,
    Tan,
    Sqrt,
    Abs,
    End,
}

#[derive(Clone, Copy)]
enum Operator {
    LeftParenthesis,
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
    Negate,
    Sin,
    Cos,
    Tan,
    Sqrt,
    Abs,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParseError {
    EmptyExpression,
    ExpressionTooLong,
    InvalidCharacter,
    InvalidNumber,
    UnknownFunction,
    MissingOperand,
    MissingOperator,
    MissingOpeningParenthesis,
    MissingClosingParenthesis,
    OperatorStackOverflow,
    BytecodeTooLarge,
    EvaluationStackOverflow,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EvaluationError {
    StackUnderflow,
    StackOverflow,
    InvalidBytecode,
    NonFiniteResult,
}

pub struct CompiledExpression {
    code: [Instruction; MAX_BYTECODE_LENGTH],
    length: u8,
}

impl CompiledExpression {
    pub fn compile(source: &str) -> Result<Self, ParseError> {
        if source.len() > MAX_EXPRESSION_LENGTH {
            return Err(ParseError::ExpressionTooLong);
        }

        let mut parser = Parser::new(source.as_bytes());
        parser.parse()?;
        validate_bytecode(&parser.code, parser.code_length)?;
        Ok(Self {
            code: parser.code,
            length: parser.code_length as u8,
        })
    }

    pub fn evaluate_checked(&self, x: f32, y: f32) -> Result<f32, EvaluationError> {
        let mut stack = [0.0_f32; MAX_EVALUATION_DEPTH];
        let mut depth = 0_usize;
        let mut position = 0_usize;

        while position < self.length as usize {
            match self.code[position] {
                Instruction::Constant(value) => push(&mut stack, &mut depth, value)?,
                Instruction::X => push(&mut stack, &mut depth, x)?,
                Instruction::Y => push(&mut stack, &mut depth, y)?,
                Instruction::Negate => apply_unary(&mut stack, depth, |value| -value)?,
                Instruction::Sin => apply_unary(&mut stack, depth, |value| math::sin_cos(value).0)?,
                Instruction::Cos => apply_unary(&mut stack, depth, |value| math::sin_cos(value).1)?,
                Instruction::Tan => apply_unary(&mut stack, depth, math::tan)?,
                Instruction::Sqrt => apply_unary(&mut stack, depth, math::sqrt)?,
                Instruction::Abs => apply_unary(&mut stack, depth, |value| value.abs())?,
                Instruction::Add => apply_binary(&mut stack, &mut depth, |a, b| a + b)?,
                Instruction::Subtract => apply_binary(&mut stack, &mut depth, |a, b| a - b)?,
                Instruction::Multiply => apply_binary(&mut stack, &mut depth, |a, b| a * b)?,
                Instruction::Divide => apply_binary(&mut stack, &mut depth, |a, b| a / b)?,
                Instruction::Power => apply_binary(&mut stack, &mut depth, math::pow)?,
                Instruction::End => return Err(EvaluationError::InvalidBytecode),
            }
            position += 1;
        }

        if depth != 1 {
            return Err(EvaluationError::InvalidBytecode);
        }
        if !stack[0].is_finite() {
            return Err(EvaluationError::NonFiniteResult);
        }
        Ok(stack[0])
    }
}

impl SurfaceFunction for CompiledExpression {
    fn evaluate(&self, x: f32, y: f32) -> f32 {
        match self.evaluate_checked(x, y) {
            Ok(value) => value,
            Err(_) => f32::NAN,
        }
    }
}

struct Parser<'a> {
    source: &'a [u8],
    position: usize,
    code: [Instruction; MAX_BYTECODE_LENGTH],
    code_length: usize,
    operators: [Operator; MAX_OPERATOR_DEPTH],
    operator_length: usize,
    expects_operand: bool,
    saw_value: bool,
}

impl<'a> Parser<'a> {
    fn new(source: &'a [u8]) -> Self {
        Self {
            source,
            position: 0,
            code: [Instruction::End; MAX_BYTECODE_LENGTH],
            code_length: 0,
            operators: [Operator::LeftParenthesis; MAX_OPERATOR_DEPTH],
            operator_length: 0,
            expects_operand: true,
            saw_value: false,
        }
    }

    fn parse(&mut self) -> Result<(), ParseError> {
        while self.position < self.source.len() {
            self.skip_spaces();
            if self.position >= self.source.len() {
                break;
            }

            let byte = self.source[self.position];
            if byte.is_ascii_digit() || byte == b'.' {
                if !self.expects_operand {
                    return Err(ParseError::MissingOperator);
                }
                let value = self.parse_number()?;
                self.emit(Instruction::Constant(value))?;
                self.accept_value();
            } else if byte.is_ascii_alphabetic() {
                self.parse_identifier()?;
            } else {
                self.parse_symbol(byte)?;
            }
        }

        if !self.saw_value {
            return Err(ParseError::EmptyExpression);
        }
        if self.expects_operand {
            return Err(ParseError::MissingOperand);
        }
        while self.operator_length > 0 {
            let operator = self.pop_operator();
            if matches!(operator, Operator::LeftParenthesis) {
                return Err(ParseError::MissingClosingParenthesis);
            }
            self.emit_operator(operator)?;
        }
        Ok(())
    }

    fn parse_number(&mut self) -> Result<f32, ParseError> {
        let mut value = 0.0_f32;
        let mut digits = 0_usize;
        while self.position < self.source.len() && self.source[self.position].is_ascii_digit() {
            value = value * 10.0 + (self.source[self.position] - b'0') as f32;
            self.position += 1;
            digits += 1;
        }

        if self.position < self.source.len() && self.source[self.position] == b'.' {
            self.position += 1;
            let mut place = 0.1_f32;
            while self.position < self.source.len() && self.source[self.position].is_ascii_digit() {
                value += (self.source[self.position] - b'0') as f32 * place;
                place *= 0.1;
                self.position += 1;
                digits += 1;
            }
        }
        if digits == 0 || !value.is_finite() {
            return Err(ParseError::InvalidNumber);
        }

        if self.position < self.source.len()
            && (self.source[self.position] == b'e' || self.source[self.position] == b'E')
        {
            self.position += 1;
            let mut negative = false;
            if self.position < self.source.len()
                && (self.source[self.position] == b'+' || self.source[self.position] == b'-')
            {
                negative = self.source[self.position] == b'-';
                self.position += 1;
            }
            let mut exponent = 0_u16;
            let mut exponent_digits = 0_usize;
            while self.position < self.source.len() && self.source[self.position].is_ascii_digit() {
                exponent = exponent
                    .saturating_mul(10)
                    .saturating_add((self.source[self.position] - b'0') as u16);
                self.position += 1;
                exponent_digits += 1;
            }
            if exponent_digits == 0 || exponent > 38 {
                return Err(ParseError::InvalidNumber);
            }
            let factor = integer_power(10.0, exponent as u32);
            value = if negative {
                value / factor
            } else {
                value * factor
            };
        }

        if value.is_finite() {
            Ok(value)
        } else {
            Err(ParseError::InvalidNumber)
        }
    }

    fn parse_identifier(&mut self) -> Result<(), ParseError> {
        if !self.expects_operand {
            return Err(ParseError::MissingOperator);
        }
        let start = self.position;
        while self.position < self.source.len() && self.source[self.position].is_ascii_alphabetic()
        {
            self.position += 1;
        }
        let name = &self.source[start..self.position];
        if name == b"x" {
            self.emit(Instruction::X)?;
            self.accept_value();
            return Ok(());
        }
        if name == b"y" {
            self.emit(Instruction::Y)?;
            self.accept_value();
            return Ok(());
        }

        let function = if name == b"sin" {
            Operator::Sin
        } else if name == b"cos" {
            Operator::Cos
        } else if name == b"tan" {
            Operator::Tan
        } else if name == b"sqrt" {
            Operator::Sqrt
        } else if name == b"abs" {
            Operator::Abs
        } else {
            return Err(ParseError::UnknownFunction);
        };

        self.skip_spaces();
        if self.position >= self.source.len() || self.source[self.position] != b'(' {
            return Err(ParseError::MissingOpeningParenthesis);
        }
        self.push_operator(function)?;
        self.push_operator(Operator::LeftParenthesis)?;
        self.position += 1;
        Ok(())
    }

    fn parse_symbol(&mut self, byte: u8) -> Result<(), ParseError> {
        self.position += 1;
        match byte {
            b'(' => {
                if !self.expects_operand {
                    return Err(ParseError::MissingOperator);
                }
                self.push_operator(Operator::LeftParenthesis)
            }
            b')' => self.close_parenthesis(),
            b'-' if self.expects_operand => self.push_operator(Operator::Negate),
            b'+' if self.expects_operand => Err(ParseError::MissingOperand),
            b'+' => self.accept_binary(Operator::Add),
            b'-' => self.accept_binary(Operator::Subtract),
            b'*' => self.accept_binary(Operator::Multiply),
            b'/' => self.accept_binary(Operator::Divide),
            b'^' => self.accept_binary(Operator::Power),
            _ => Err(ParseError::InvalidCharacter),
        }
    }

    fn close_parenthesis(&mut self) -> Result<(), ParseError> {
        if self.expects_operand {
            return Err(ParseError::MissingOperand);
        }
        let mut found = false;
        while self.operator_length > 0 {
            let operator = self.pop_operator();
            if matches!(operator, Operator::LeftParenthesis) {
                found = true;
                break;
            }
            self.emit_operator(operator)?;
        }
        if !found {
            return Err(ParseError::MissingOpeningParenthesis);
        }
        if self.operator_length > 0 {
            let operator = self.operators[self.operator_length - 1];
            if is_function(operator) {
                self.operator_length -= 1;
                self.emit_operator(operator)?;
            }
        }
        self.expects_operand = false;
        Ok(())
    }

    fn accept_binary(&mut self, operator: Operator) -> Result<(), ParseError> {
        if self.expects_operand {
            return Err(ParseError::MissingOperand);
        }
        while self.operator_length > 0 {
            let top = self.operators[self.operator_length - 1];
            if matches!(top, Operator::LeftParenthesis) || is_function(top) {
                break;
            }
            let should_pop = precedence(top) > precedence(operator)
                || (precedence(top) == precedence(operator) && !is_right_associative(operator));
            if !should_pop {
                break;
            }
            self.operator_length -= 1;
            self.emit_operator(top)?;
        }
        self.push_operator(operator)?;
        self.expects_operand = true;
        Ok(())
    }

    fn accept_value(&mut self) {
        self.expects_operand = false;
        self.saw_value = true;
    }

    fn emit_operator(&mut self, operator: Operator) -> Result<(), ParseError> {
        let instruction = match operator {
            Operator::Add => Instruction::Add,
            Operator::Subtract => Instruction::Subtract,
            Operator::Multiply => Instruction::Multiply,
            Operator::Divide => Instruction::Divide,
            Operator::Power => Instruction::Power,
            Operator::Negate => Instruction::Negate,
            Operator::Sin => Instruction::Sin,
            Operator::Cos => Instruction::Cos,
            Operator::Tan => Instruction::Tan,
            Operator::Sqrt => Instruction::Sqrt,
            Operator::Abs => Instruction::Abs,
            Operator::LeftParenthesis => return Err(ParseError::MissingClosingParenthesis),
        };
        self.emit(instruction)
    }

    fn emit(&mut self, instruction: Instruction) -> Result<(), ParseError> {
        if self.code_length >= MAX_BYTECODE_LENGTH {
            return Err(ParseError::BytecodeTooLarge);
        }
        self.code[self.code_length] = instruction;
        self.code_length += 1;
        Ok(())
    }

    fn push_operator(&mut self, operator: Operator) -> Result<(), ParseError> {
        if self.operator_length >= MAX_OPERATOR_DEPTH {
            return Err(ParseError::OperatorStackOverflow);
        }
        self.operators[self.operator_length] = operator;
        self.operator_length += 1;
        Ok(())
    }

    fn pop_operator(&mut self) -> Operator {
        self.operator_length -= 1;
        self.operators[self.operator_length]
    }

    fn skip_spaces(&mut self) {
        while self.position < self.source.len() && self.source[self.position].is_ascii_whitespace()
        {
            self.position += 1;
        }
    }
}

fn precedence(operator: Operator) -> u8 {
    match operator {
        Operator::Add | Operator::Subtract => 1,
        Operator::Multiply | Operator::Divide => 2,
        Operator::Negate => 3,
        Operator::Power => 4,
        _ => 0,
    }
}

fn is_right_associative(operator: Operator) -> bool {
    matches!(operator, Operator::Power | Operator::Negate)
}

fn is_function(operator: Operator) -> bool {
    matches!(
        operator,
        Operator::Sin | Operator::Cos | Operator::Tan | Operator::Sqrt | Operator::Abs
    )
}

fn validate_bytecode(
    code: &[Instruction; MAX_BYTECODE_LENGTH],
    length: usize,
) -> Result<(), ParseError> {
    let mut depth = 0_usize;
    let mut position = 0_usize;
    while position < length {
        match code[position] {
            Instruction::Constant(_) | Instruction::X | Instruction::Y => depth += 1,
            Instruction::Negate
            | Instruction::Sin
            | Instruction::Cos
            | Instruction::Tan
            | Instruction::Sqrt
            | Instruction::Abs => {
                if depth < 1 {
                    return Err(ParseError::MissingOperand);
                }
            }
            Instruction::Add
            | Instruction::Subtract
            | Instruction::Multiply
            | Instruction::Divide
            | Instruction::Power => {
                if depth < 2 {
                    return Err(ParseError::MissingOperand);
                }
                depth -= 1;
            }
            Instruction::End => return Err(ParseError::MissingOperand),
        }
        if depth > MAX_EVALUATION_DEPTH {
            return Err(ParseError::EvaluationStackOverflow);
        }
        position += 1;
    }
    if depth == 1 {
        Ok(())
    } else {
        Err(ParseError::MissingOperator)
    }
}

fn push(
    stack: &mut [f32; MAX_EVALUATION_DEPTH],
    depth: &mut usize,
    value: f32,
) -> Result<(), EvaluationError> {
    if *depth >= stack.len() {
        return Err(EvaluationError::StackOverflow);
    }
    if !value.is_finite() {
        return Err(EvaluationError::NonFiniteResult);
    }
    stack[*depth] = value;
    *depth += 1;
    Ok(())
}

fn apply_unary<F>(
    stack: &mut [f32; MAX_EVALUATION_DEPTH],
    depth: usize,
    operation: F,
) -> Result<(), EvaluationError>
where
    F: FnOnce(f32) -> f32,
{
    if depth < 1 {
        return Err(EvaluationError::StackUnderflow);
    }
    let result = operation(stack[depth - 1]);
    if !result.is_finite() {
        return Err(EvaluationError::NonFiniteResult);
    }
    stack[depth - 1] = result;
    Ok(())
}

fn apply_binary<F>(
    stack: &mut [f32; MAX_EVALUATION_DEPTH],
    depth: &mut usize,
    operation: F,
) -> Result<(), EvaluationError>
where
    F: FnOnce(f32, f32) -> f32,
{
    if *depth < 2 {
        return Err(EvaluationError::StackUnderflow);
    }
    let right = stack[*depth - 1];
    let left = stack[*depth - 2];
    let result = operation(left, right);
    if !result.is_finite() {
        return Err(EvaluationError::NonFiniteResult);
    }
    *depth -= 1;
    stack[*depth - 1] = result;
    Ok(())
}

fn integer_power(mut base: f32, mut exponent: u32) -> f32 {
    let mut result = 1.0_f32;
    while exponent > 0 {
        if exponent & 1 != 0 {
            result *= base;
        }
        base *= base;
        exponent >>= 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(source: &str, x: f32, y: f32) -> f32 {
        match CompiledExpression::compile(source) {
            Ok(expression) => match expression.evaluate_checked(x, y) {
                Ok(result) => result,
                Err(error) => panic!("evaluation failed: {:?}", error),
            },
            Err(error) => panic!("parse failed: {:?}", error),
        }
    }

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.002,
            "{} != {}",
            actual,
            expected
        );
    }

    #[test]
    fn constants_and_variables() {
        close(value("2.5", 0.0, 0.0), 2.5);
        close(value("x + y", 1.25, 2.5), 3.75);
    }

    #[test]
    fn precedence_and_parentheses() {
        close(value("2 + 3 * 4", 0.0, 0.0), 14.0);
        close(value("(2 + 3) * 4", 0.0, 0.0), 20.0);
    }

    #[test]
    fn unary_minus_and_power_associativity() {
        close(value("-2^2", 0.0, 0.0), -4.0);
        close(value("(-2)^2", 0.0, 0.0), 4.0);
        close(value("2^3^2", 0.0, 0.0), 512.0);
        close(value("2^-2", 0.0, 0.0), 0.25);
        close(value("-x * y", 2.0, 3.0), -6.0);
    }

    #[test]
    fn functions_and_examples() {
        close(value("sin(x) * cos(y)", 0.5, 0.25), 0.46452);
        close(value("x^2 + y^2", 3.0, 4.0), 25.0);
        close(value("sqrt(x^2 + y^2)", 3.0, 4.0), 5.0);
        close(value("sin(sqrt(x^2 + y^2))", 0.3, 0.4), 0.47943);
        close(value("abs(-3) + tan(0)", 0.0, 0.0), 3.0);
    }

    #[test]
    fn malformed_expressions_are_rejected() {
        assert!(matches!(
            CompiledExpression::compile(""),
            Err(ParseError::EmptyExpression)
        ));
        assert!(CompiledExpression::compile("2 +").is_err());
        assert!(CompiledExpression::compile("(x + y").is_err());
        assert!(CompiledExpression::compile("x y").is_err());
        assert!(CompiledExpression::compile("unknown(x)").is_err());
        assert!(CompiledExpression::compile("sin x").is_err());
        assert!(CompiledExpression::compile("x @ y").is_err());
    }

    #[test]
    fn invalid_math_is_reported() {
        let division = CompiledExpression::compile("1 / 0").expect("valid expression");
        assert_eq!(
            division.evaluate_checked(0.0, 0.0),
            Err(EvaluationError::NonFiniteResult)
        );
        let square_root = CompiledExpression::compile("sqrt(-1)").expect("valid expression");
        assert_eq!(
            square_root.evaluate_checked(0.0, 0.0),
            Err(EvaluationError::NonFiniteResult)
        );
    }

    #[test]
    fn fixed_capacity_limits_are_reported() {
        let too_long = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
        assert!(matches!(
            CompiledExpression::compile(too_long),
            Err(ParseError::ExpressionTooLong)
        ));

        let mut code = [Instruction::End; MAX_BYTECODE_LENGTH];
        let mut position = 0;
        while position <= MAX_EVALUATION_DEPTH {
            code[position] = Instruction::X;
            position += 1;
        }
        let invalid = CompiledExpression {
            code,
            length: (MAX_EVALUATION_DEPTH + 1) as u8,
        };
        assert_eq!(
            invalid.evaluate_checked(1.0, 1.0),
            Err(EvaluationError::StackOverflow)
        );
    }
}
