//! Internal `returnType` values the compiler's tag lowerings pass to
//! `queryExecute`. The compiler writes them and the query drivers read them, so
//! both take them from here.

/// Every result set of the statement, as an array of queries. SQL Server and
/// MySQL return them all; a driver with one result set returns it in a
/// one-element array. Asked for by the `<cfstoredproc>` lowerings.
pub const RESULT_SETS_RETURN_TYPE: &str = "__resultsets";

/// A `<cfstoredproc>` with OUT parameters or a return code:
/// `{ resultSets: [query, …], outputs: { variableName: value, … }, statusCode }`.
pub const PROC_RESULT_RETURN_TYPE: &str = "__procresult";

/// [`PROC_RESULT_RETURN_TYPE`], also capturing the procedure's return value
/// (`returnCode="true"`).
pub const PROC_RESULT_RC_RETURN_TYPE: &str = "__procresult:rc";
