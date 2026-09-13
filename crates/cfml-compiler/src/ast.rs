//! CFML Abstract Syntax Tree

use cfml_common::position::SourceLocation;

#[derive(Debug, Clone)]
pub enum CfmlNode {
    Program(Program),
    Template(Template),
    Component(Component),
    Function(Function),
    Statement(Statement),
    Expression(Expression),
}

#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<CfmlNode>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Template {
    pub name: Option<String>,
    pub body: Vec<Statement>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Component {
    pub name: String,
    pub extends: Option<String>,
    pub implements: Vec<String>,
    pub properties: Vec<Property>,
    pub functions: Vec<Function>,
    pub body: Vec<Statement>,
    /// Statements inside a `static { ... }` initialization block. Executed once
    /// per component type (on first load), populating the shared `static` scope.
    pub static_body: Vec<Statement>,
    pub location: SourceLocation,
    pub metadata: Vec<(String, String)>,
    pub accessors: bool,
}

#[derive(Debug, Clone)]
pub struct Interface {
    pub name: String,
    pub extends: Vec<String>,
    pub functions: Vec<Function>,
    pub metadata: Vec<(String, String)>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Property {
    pub name: String,
    pub prop_type: Option<String>,
    pub default: Option<Expression>,
    pub required: bool,
    pub attributes: Vec<(String, String)>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<String>,
    pub access: AccessModifier,
    pub is_static: bool,
    pub is_abstract: bool,
    pub body: Vec<Statement>,
    pub location: SourceLocation,
    pub metadata: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub param_type: Option<String>,
    pub default: Option<Expression>,
    pub required: bool,
    /// Javadoc-style `@paramname.key value` annotations attached to this
    /// parameter (e.g. WireBox `@configuredFeatures.inject coldbox:setting:features`).
    pub annotations: Vec<(String, String)>,
}

impl Param {
    /// The value `getMetaData()` reports for this parameter's `default` key, or
    /// `None` when the parameter has no default (the key is then absent, as it
    /// is on Lucee).
    ///
    /// A LITERAL default is reported as its value, preserving the declared type
    /// — `someParam="test"` reports the string `test`, `anotherParam=false` the
    /// boolean `false`, `b=7` the number `7`. Anything the engine must evaluate
    /// at call time (`a=now()`, `b=[1,2]`, `c={x:1}`) has no value to report at
    /// reflection time, and Lucee reports the fixed marker string
    /// `[runtime expression]` rather than the source text. Verified against
    /// Lucee 7.1.0.204 (GH #399).
    pub fn metadata_default(&self) -> Option<cfml_common::dynamic::CfmlValue> {
        use cfml_common::dynamic::CfmlValue;
        let expr = self.default.as_ref()?;
        Some(match expr {
            Expression::Literal(lit) => match &lit.value {
                LiteralValue::Null => CfmlValue::Null,
                LiteralValue::Bool(b) => CfmlValue::Bool(*b),
                LiteralValue::Int(i) => CfmlValue::Int(*i),
                LiteralValue::Double(d) => CfmlValue::Double(*d),
                LiteralValue::String(s) => CfmlValue::string(s.clone()),
            },
            _ => CfmlValue::string("[runtime expression]".to_string()),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessModifier {
    Public,
    Private,
    Package,
    Remote,
}

#[derive(Debug, Clone)]
pub enum Statement {
    Expression(ExpressionStatement),
    Assignment(Assignment),
    Return(Return),
    If(If),
    Switch(Switch),
    For(For),
    ForIn(ForIn),
    While(While),
    Do(Do),
    Break(Break),
    Continue(Continue),
    Try(Try),
    Throw(Throw),
    Rethrow(SourceLocation),
    Import(Import),
    Var(Var),
    ComponentDecl(ComponentDecl),
    InterfaceDecl(InterfaceDecl),
    PropertyDecl(PropertyDecl),
    FunctionDecl(FunctionDecl),
    Output(Output),
    Include(Include),
    Exit,
}

#[derive(Debug, Clone)]
pub struct ExpressionStatement {
    pub expr: Expression,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Assignment {
    pub target: AssignTarget,
    pub value: Expression,
    pub operator: AssignOp,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub enum AssignTarget {
    Variable(String),
    ArrayAccess(Box<Expression>, Box<Expression>),
    StructAccess(Box<Expression>, String),
}

#[derive(Debug, Clone, Copy)]
pub enum AssignOp {
    Equal,
    PlusEqual,
    MinusEqual,
    StarEqual,
    SlashEqual,
    PercentEqual,
    ConcatEqual,
}

#[derive(Debug, Clone)]
pub struct Return {
    pub value: Option<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct If {
    pub condition: Expression,
    pub then_branch: Vec<Statement>,
    pub else_if: Vec<ElseIf>,
    pub else_branch: Option<Vec<Statement>>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct ElseIf {
    pub condition: Expression,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub struct Switch {
    pub expression: Expression,
    pub cases: Vec<SwitchCase>,
    pub default_case: Option<Vec<Statement>>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub values: Vec<Expression>,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub struct For {
    pub init: Option<Box<Statement>>,
    pub condition: Option<Expression>,
    pub increment: Option<Box<Expression>>,
    pub body: Vec<Statement>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct ForIn {
    pub variable: String,
    /// `for (var x in …)`: the loop variable is declared function-local.
    /// Without it the variable is an ordinary unscoped write (routed like
    /// `x = …`: `variables` in a classic method, the page scope at page level).
    pub var_declared: bool,
    pub iterable: Expression,
    pub body: Vec<Statement>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct While {
    pub condition: Expression,
    pub body: Vec<Statement>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Do {
    pub body: Vec<Statement>,
    pub condition: Expression,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Break {
    pub label: Option<String>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Continue {
    pub label: Option<String>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Try {
    pub body: Vec<Statement>,
    pub catches: Vec<Catch>,
    pub finally_body: Option<Vec<Statement>>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Catch {
    pub var_type: Option<String>,
    pub var_name: String,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub struct Throw {
    pub message: Option<Expression>,
    pub type_: Option<String>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Import {
    pub path: String,
    pub alias: Option<String>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Var {
    pub name: String,
    pub value: Option<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct ComponentDecl {
    pub component: Component,
}

#[derive(Debug, Clone)]
pub struct InterfaceDecl {
    pub interface: Interface,
}

#[derive(Debug, Clone)]
pub struct PropertyDecl {
    pub prop: Property,
}

#[derive(Debug, Clone)]
pub struct FunctionDecl {
    pub func: Function,
}

#[derive(Debug, Clone)]
pub struct Output {
    pub body: Vec<Statement>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Include {
    pub path: Expression,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub enum Expression {
    Literal(Literal),
    Identifier(Identifier),
    Array(Array),
    Struct(Struct),
    FunctionCall(Box<FunctionCall>),
    MethodCall(Box<MethodCall>),
    StaticCall(Box<StaticCall>),
    StaticMember(Box<StaticMember>),
    MemberAccess(Box<MemberAccess>),
    ArrayAccess(Box<ArrayAccess>),
    UnaryOp(Box<UnaryOp>),
    BinaryOp(Box<BinaryOp>),
    Ternary(Box<Ternary>),
    New(Box<NewExpression>),
    Closure(Box<Closure>),
    ArrowFunction(Box<ArrowFunction>),
    This(This),
    Super(Super),
    PostfixOp(Box<PostfixOp>),
    StringInterpolation(StringInterpolation),
    Elvis(Box<Elvis>),
    Spread(Box<Expression>),
    NamedArgument(Box<NamedArgument>),
    Empty,
}

#[derive(Debug, Clone)]
pub struct StringInterpolation {
    pub parts: Vec<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct NamedArgument {
    pub name: String,
    pub value: Box<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Elvis {
    pub left: Box<Expression>,
    pub right: Box<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Literal {
    pub value: LiteralValue,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub enum LiteralValue {
    Null,
    Bool(bool),
    Int(i64),
    Double(f64),
    String(String),
}

#[derive(Debug, Clone)]
pub struct Identifier {
    pub name: String,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Array {
    pub elements: Vec<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Struct {
    pub pairs: Vec<(Expression, Expression)>,
    pub ordered: bool, // true for ordered structs [:]
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct FunctionCall {
    pub name: Box<Expression>,
    pub arguments: Vec<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct MethodCall {
    pub object: Box<Expression>,
    pub method: String,
    pub arguments: Vec<Expression>,
    pub null_safe: bool,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct StaticCall {
    pub class: Box<Expression>,
    pub method: String,
    pub arguments: Vec<Expression>,
    pub location: SourceLocation,
}

/// `Component::member` — read a static member of a component type without an
/// instance (the `::` operator). The method-call form `Component::fn(...)` is
/// `StaticCall`.
#[derive(Debug, Clone)]
pub struct StaticMember {
    pub class: Box<Expression>,
    pub member: String,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct MemberAccess {
    pub object: Box<Expression>,
    pub member: String,
    pub null_safe: bool,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct ArrayAccess {
    pub array: Box<Expression>,
    pub index: Box<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct UnaryOp {
    pub operator: UnaryOpType,
    pub operand: Box<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone, Copy)]
pub enum UnaryOpType {
    Minus,
    Not,
    BitNot,
    PrefixIncrement,
    PrefixDecrement,
}

#[derive(Debug, Clone)]
pub struct BinaryOp {
    pub left: Box<Expression>,
    pub operator: BinaryOpType,
    pub right: Box<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOpType {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    IntDiv,
    // String
    Concat,
    // Comparison
    Equal,
    NotEqual,
    /// `===` / `!==` strict (same-type) equality — no cross-type coercion.
    StrictEqual,
    StrictNotEqual,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    // Logical
    And,
    Or,
    Xor,
    // CFML-specific
    Contains,
    DoesNotContain,
    Eqv,
    Imp,
    // Assignment (used internally)
    Assign,
}

#[derive(Debug, Clone)]
pub struct Ternary {
    pub condition: Box<Expression>,
    pub then_expr: Box<Expression>,
    pub else_expr: Box<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct NewExpression {
    pub class: Box<Expression>,
    pub arguments: Vec<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Closure {
    pub params: Vec<Param>,
    pub body: Vec<Statement>,
    pub location: SourceLocation,
    /// Function attributes parsed between `)` and `{` — e.g.
    /// `function (x) localMode="modern" { ... }`.
    pub metadata: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct ArrowFunction {
    pub params: Vec<Param>,
    pub body: Box<Expression>,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct PostfixOp {
    pub operand: Box<Expression>,
    pub operator: PostfixOpType,
    pub location: SourceLocation,
}

#[derive(Debug, Clone, Copy)]
pub enum PostfixOpType {
    Increment,
    Decrement,
}

#[derive(Debug, Clone)]
pub struct This {
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Super {
    pub location: SourceLocation,
}
