//! CFML Standard Library

pub mod builtins;
#[cfg(feature = "image_support")]
pub mod image;
#[cfg(feature = "spreadsheet")]
pub mod spreadsheet;
#[cfg(feature = "xml")]
pub mod xmp;
#[cfg(feature = "html")]
pub mod html_dom;
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

pub use builtins::*;
pub use db_driver::{
    DynamicDbDriver, has_dynamic_datasource, lookup_dynamic_datasource,
    register_dynamic_datasource, unregister_dynamic_datasource,
};
