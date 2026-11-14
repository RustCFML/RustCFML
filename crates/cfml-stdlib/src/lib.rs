//! CFML Standard Library

pub mod builtins;
pub mod dates;
pub mod file_handles;
#[cfg(feature = "image_support")]
pub mod image;
#[cfg(feature = "image_support")]
pub mod qr_zxing;
#[cfg(feature = "spreadsheet")]
pub mod spreadsheet;
#[cfg(feature = "xml")]
pub mod xmp;
#[cfg(feature = "html")]
pub mod html_dom;
#[cfg(feature = "markdown")]
pub mod markdown;
#[cfg(feature = "pdf")]
pub mod pdf;
pub mod db_driver;
pub mod pg_sql;
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub mod dbinfo;
#[cfg(feature = "s3")]
pub mod s3;
#[cfg(feature = "s3")]
pub mod s3_builtins;
#[cfg(feature = "s3")]
pub mod s3_service;

pub use builtins::*;
pub use db_driver::{
    DynamicDbDriver, DynamicTransaction, has_dynamic_datasource, lookup_dynamic_datasource,
    register_dynamic_datasource, unregister_dynamic_datasource,
};
/// Result shaping, for drivers supplied by an extension — see `db_driver`.
pub use builtins::{build_mutation_result, build_query_result_columnar, dedup_result_columns};
