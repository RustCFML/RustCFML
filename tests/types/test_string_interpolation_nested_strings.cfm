<cfscript>
suiteBegin("Type: string interpolation nested string literals");

flag = true;
sameQuoteTernary = "value=#flag ? "YES" : ""#";
assert("double-quoted outer string allows double-quoted ternary branches", sameQuoteTernary, "value=YES");

flag = false;
sameQuoteEmptyBranch = "value=#flag ? "YES" : ""#";
assert("same-quote ternary empty-string branch", sameQuoteEmptyBranch, "value=");

indexName = "idx_demo";
statementStruct = {
    statement: "CREATE #flag ? "UNIQUE" : ""# INDEX #indexName#"
};
assert("same-quote interpolation inside struct literal value", statementStruct.statement, "CREATE  INDEX idx_demo");

flag = true;
statementStruct.statement = "CREATE #flag ? "UNIQUE" : ""# INDEX #indexName#";
assert("same-quote interpolation preserves following interpolation segments", statementStruct.statement, "CREATE UNIQUE INDEX idx_demo");

quotedValue = "value=#flag ? "A ""quoted"" value" : "fallback"#";
assert("same-quote nested string keeps doubled quote escapes", quotedValue, "value=A ""quoted"" value");

status = "active";
keywordOperator = "enabled=#status EQ "active" ? "yes" : "no"#";
assert("keyword operator followed by same-quote string literal", keywordOperator, "enabled=yes");

singleOuterFlag = true;
singleQuotedOuter = 'value=#singleOuterFlag ? 'YES' : ''#';
assert("single-quoted outer string allows single-quoted ternary branches", singleQuotedOuter, "value=YES");

suiteEnd();
</cfscript>
