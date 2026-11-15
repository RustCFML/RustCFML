//! What a statement run inside `transaction { }` does to the database
//! transaction underneath it.
//!
//! Lucee runs a `cftransaction` with JDBC autocommit OFF. A raw `COMMIT` or
//! `ROLLBACK` sent through `queryExecute` (Preside's PostgreSQL adapter sends
//! one after every insert) ends the database transaction, and the driver
//! starts a new one before the next statement, so the `cftransaction` stays
//! usable to its end. RustCFML opens the transaction with an explicit `BEGIN`
//! instead, so it has to notice when user SQL closed it and begin again
//! before the next statement (GH #536). MySQL also commits implicitly before
//! most DDL; Lucee carries on transactionally after that too.
//!
//! Only the first statement of the SQL is classified. A multi-statement string
//! that ends a transaction part-way through is not tracked.

/// The SQL dialect of the connection the statement runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxnDialect {
    Sqlite,
    Mysql,
    Postgres,
    Mssql,
}

/// The effect a statement has on the open/closed state of the transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxnEffect {
    /// The statement starts a transaction (`BEGIN`, `START TRANSACTION`).
    Opens,
    /// The statement ends the transaction, explicitly or implicitly.
    Closes,
    /// The statement leaves the transaction as it was.
    Neutral,
}

/// Classify `sql` for `dialect`.
pub fn txn_effect(sql: &str, dialect: TxnDialect) -> TxnEffect {
    // Runs on every statement inside a transaction, so it allocates nothing.
    let words = LeadingWords::<4>::read(sql);
    let w = |i: usize| words.get(i);
    let has_and_chain = |from: usize| (from..3).any(|i| w(i) == "AND" && w(i + 1) == "CHAIN");
    match dialect {
        TxnDialect::Postgres => match w(0) {
            "BEGIN" => TxnEffect::Opens,
            "START" if w(1) == "TRANSACTION" => TxnEffect::Opens,
            // `COMMIT PREPARED 'x'` / `ROLLBACK PREPARED 'x'` finish a
            // two-phase transaction and run outside a transaction block.
            "COMMIT" | "ROLLBACK" if w(1) == "PREPARED" => TxnEffect::Neutral,
            "COMMIT" | "END" | "ROLLBACK" | "ABORT" => {
                if w(1) == "TO" || (matches!(w(1), "WORK" | "TRANSACTION") && w(2) == "TO") {
                    // ROLLBACK [WORK|TRANSACTION] TO [SAVEPOINT] x
                    TxnEffect::Neutral
                } else if has_and_chain(1) {
                    TxnEffect::Neutral
                } else {
                    TxnEffect::Closes
                }
            }
            "PREPARE" if w(1) == "TRANSACTION" => TxnEffect::Closes,
            _ => TxnEffect::Neutral,
        },
        TxnDialect::Mysql => match w(0) {
            // `BEGIN NOT ATOMIC` opens a compound statement, not a transaction.
            "BEGIN" if w(1) == "NOT" => TxnEffect::Neutral,
            "BEGIN" => TxnEffect::Opens,
            "START" if w(1) == "TRANSACTION" => TxnEffect::Opens,
            "COMMIT" | "ROLLBACK" => {
                if w(1) == "TO" || (w(1) == "WORK" && w(2) == "TO") || has_and_chain(1) {
                    TxnEffect::Neutral
                } else {
                    TxnEffect::Closes
                }
            }
            // Statements that cause an implicit commit
            // (https://dev.mysql.com/doc/refman/8.4/en/implicit-commit.html).
            // TEMPORARY tables are the documented exception.
            "CREATE" | "DROP" if w(1) == "TEMPORARY" => TxnEffect::Neutral,
            "ALTER" | "CREATE" | "DROP" | "RENAME" | "TRUNCATE" | "GRANT" | "REVOKE"
            | "ANALYZE" | "OPTIMIZE" | "REPAIR" | "FLUSH" => TxnEffect::Closes,
            "LOCK" if matches!(w(1), "TABLES" | "TABLE" | "INSTANCE") => TxnEffect::Closes,
            _ => TxnEffect::Neutral,
        },
        TxnDialect::Mssql => match w(0) {
            // A bare T-SQL `BEGIN` opens a statement block.
            "BEGIN" if matches!(w(1), "TRAN" | "TRANSACTION") => TxnEffect::Opens,
            "BEGIN" if w(1) == "DISTRIBUTED" => TxnEffect::Opens,
            "COMMIT" => TxnEffect::Closes,
            "ROLLBACK" => {
                // `ROLLBACK TRAN name` names a savepoint (or a transaction);
                // only a bare rollback is certain to end the transaction.
                let name_at = if matches!(w(1), "TRAN" | "TRANSACTION" | "WORK") { 2 } else { 1 };
                if rest_has_name(sql, name_at) {
                    TxnEffect::Neutral
                } else {
                    TxnEffect::Closes
                }
            }
            _ => TxnEffect::Neutral,
        },
        TxnDialect::Sqlite => match w(0) {
            "BEGIN" => TxnEffect::Opens,
            "COMMIT" | "END" => TxnEffect::Closes,
            "ROLLBACK" => {
                if w(1) == "TO" || (w(1) == "TRANSACTION" && w(2) == "TO") {
                    TxnEffect::Neutral
                } else {
                    TxnEffect::Closes
                }
            }
            _ => TxnEffect::Neutral,
        },
    }
}

/// True for a SQL Server statement that controls the transaction: BEGIN /
/// COMMIT / ROLLBACK / SAVE TRANSACTION. These must run as a plain batch, not
/// inside sp_executesql, which refuses a change to @@TRANCOUNT.
pub fn is_mssql_txn_control(sql: &str) -> bool {
    let words = LeadingWords::<2>::read(sql);
    let w = |i: usize| words.get(i);
    match w(0) {
        "COMMIT" | "ROLLBACK" => true,
        "BEGIN" | "SAVE" => matches!(w(1), "TRAN" | "TRANSACTION" | "DISTRIBUTED"),
        _ => false,
    }
}

/// The longest keyword compared against (`TRANSACTION`, `DISTRIBUTED`).
const MAX_KEYWORD: usize = 11;

/// The first `N` words of `sql`, upper-cased in place, after whitespace and
/// comments, stopping at the first token that is not a plain word. A word
/// longer than any keyword reads as `"?"`, which matches nothing.
struct LeadingWords<const N: usize> {
    words: [[u8; MAX_KEYWORD]; N],
    lens: [usize; N],
    count: usize,
}

impl<const N: usize> LeadingWords<N> {
    fn read(sql: &str) -> Self {
        let mut out = LeadingWords { words: [[0; MAX_KEYWORD]; N], lens: [0; N], count: 0 };
        let mut s = skip_noise(sql);
        while out.count < N {
            let len = s.bytes().take_while(|b| b.is_ascii_alphabetic() || *b == b'_').count();
            if len == 0 {
                break;
            }
            if len <= MAX_KEYWORD {
                for (dst, b) in out.words[out.count].iter_mut().zip(s.bytes()) {
                    *dst = b.to_ascii_uppercase();
                }
                out.lens[out.count] = len;
            } else {
                out.words[out.count][0] = b'?';
                out.lens[out.count] = 1;
            }
            out.count += 1;
            s = skip_noise(&s[len..]);
        }
        out
    }

    fn get(&self, i: usize) -> &str {
        if i >= self.count {
            return "";
        }
        // ASCII letters and `_` only, so always valid UTF-8.
        std::str::from_utf8(&self.words[i][..self.lens[i]]).unwrap_or("")
    }
}

/// True when a token other than a terminator follows the first `skip` words.
fn rest_has_name(sql: &str, skip: usize) -> bool {
    let mut s = skip_noise(sql);
    for _ in 0..skip {
        let len = s.bytes().take_while(|b| b.is_ascii_alphabetic() || *b == b'_').count();
        s = skip_noise(&s[len..]);
    }
    !(s.is_empty() || s.starts_with(';'))
}

fn skip_noise(sql: &str) -> &str {
    let mut s = sql.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
            s = match rest.find('\n') {
                Some(i) => rest[i + 1..].trim_start(),
                None => return "",
            };
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = match rest.find("*/") {
                Some(i) => rest[i + 2..].trim_start(),
                None => return "",
            };
        } else {
            return s;
        }
    }
}

/// Quote a savepoint name as an identifier for `dialect`. Names come from CFML
/// code (`<cftransaction action="setsavepoint" savepoint="…">`), so they are
/// quoted rather than spliced into the statement as written.
pub fn quote_savepoint(name: &str, dialect: TxnDialect) -> String {
    match dialect {
        TxnDialect::Mysql => format!("`{}`", name.replace('`', "``")),
        TxnDialect::Mssql => format!("[{}]", name.replace(']', "]]")),
        TxnDialect::Postgres | TxnDialect::Sqlite => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TxnDialect::*;
    use TxnEffect::*;

    #[test]
    fn postgres_commit_rollback_and_synonyms_close() {
        for sql in ["commit", "COMMIT;", "  -- x\n commit work", "end", "END TRANSACTION", "rollback", "abort", "ROLLBACK AND NO CHAIN"] {
            assert_eq!(txn_effect(sql, Postgres), Closes, "{sql}");
        }
    }

    #[test]
    fn postgres_savepoint_chain_and_prepared_forms_stay_open() {
        for sql in ["rollback to savepoint a", "ROLLBACK TO a", "rollback work to a", "commit and chain", "COMMIT PREPARED 'x'", "rollback prepared 'x'", "savepoint a", "release savepoint a", "insert into t values (1)"] {
            assert_eq!(txn_effect(sql, Postgres), Neutral, "{sql}");
        }
        assert_eq!(txn_effect("prepare transaction 'x'", Postgres), Closes);
        assert_eq!(txn_effect("begin", Postgres), Opens);
        assert_eq!(txn_effect("start transaction isolation level serializable", Postgres), Opens);
    }

    #[test]
    fn mysql_ddl_commits_implicitly_but_temporary_tables_do_not() {
        for sql in ["create table t (x int)", "ALTER TABLE t ADD y int", "drop table t", "truncate t", "rename table a to b", "lock tables t write"] {
            assert_eq!(txn_effect(sql, Mysql), Closes, "{sql}");
        }
        for sql in ["create temporary table t (x int)", "drop temporary table t", "select 1", "rollback to savepoint a", "commit and chain", "begin not atomic select 1; end"] {
            assert_eq!(txn_effect(sql, Mysql), Neutral, "{sql}");
        }
        assert_eq!(txn_effect("start transaction", Mysql), Opens);
        assert_eq!(txn_effect("commit work", Mysql), Closes);
    }

    #[test]
    fn mssql_only_bare_rollback_closes_and_bare_begin_is_a_block() {
        assert_eq!(txn_effect("BEGIN TRAN", Mssql), Opens);
        assert_eq!(txn_effect("begin select 1 end", Mssql), Neutral);
        assert_eq!(txn_effect("commit", Mssql), Closes);
        assert_eq!(txn_effect("COMMIT TRANSACTION", Mssql), Closes);
        assert_eq!(txn_effect("rollback", Mssql), Closes);
        assert_eq!(txn_effect("ROLLBACK TRANSACTION;", Mssql), Closes);
        assert_eq!(txn_effect("rollback tran sp1", Mssql), Neutral);
        assert_eq!(txn_effect("ROLLBACK TRANSACTION [sp 1]", Mssql), Neutral);
    }

    #[test]
    fn mssql_transaction_control_is_recognised_for_the_plain_batch_path() {
        for sql in ["commit", "COMMIT TRANSACTION", "rollback tran sp1", "begin tran", "SAVE TRANSACTION sp1", "begin distributed transaction"] {
            assert!(is_mssql_txn_control(sql), "{sql}");
        }
        for sql in ["begin select 1 end", "select 1", "insert into t values (1)", "savepoint"] {
            assert!(!is_mssql_txn_control(sql), "{sql}");
        }
    }

    #[test]
    fn sqlite_end_closes_and_rollback_to_does_not() {
        assert_eq!(txn_effect("end", Sqlite), Closes);
        assert_eq!(txn_effect("rollback transaction to a", Sqlite), Neutral);
        assert_eq!(txn_effect("rollback", Sqlite), Closes);
        assert_eq!(txn_effect("begin immediate", Sqlite), Opens);
    }

    #[test]
    fn savepoint_names_are_quoted_per_dialect() {
        assert_eq!(quote_savepoint("a\"b", Postgres), "\"a\"\"b\"");
        assert_eq!(quote_savepoint("a`b", Mysql), "`a``b`");
        assert_eq!(quote_savepoint("a]b", Mssql), "[a]]b]");
    }
}
