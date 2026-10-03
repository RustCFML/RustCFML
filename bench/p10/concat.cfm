<cfscript>
col = "my_column_name"; typ = "VARCHAR(255)"; nul = " NOT NULL"; def = " DEFAULT ''"; tbl = "my_table";
n = 100000;
t0 = getTickCount("nano");
for (i=1;i<=n;i++) { sql = "ALTER TABLE `" & tbl & "` MODIFY COLUMN `" & col & "` " & typ & nul & def & " COMMENT '" & i & "'" & ", ADD INDEX `ix_" & col & "` (`" & col & "`)" & ";"; }
t1 = getTickCount("nano");
for (i=1;i<=n;i++) { sql2 = "ALTER TABLE `#tbl#` MODIFY COLUMN `#col#` #typ##nul##def# COMMENT '#i#', ADD INDEX `ix_#col#` (`#col#`);"; }
t2 = getTickCount("nano");
for (i=1;i<=n;i++) { s3 = ""; s3 &= "ALTER TABLE `"; s3 &= tbl; s3 &= "` MODIFY COLUMN `"; s3 &= col; s3 &= "` "; s3 &= typ; s3 &= nul; s3 &= def; s3 &= " COMMENT '"; s3 &= i; s3 &= "';"; }
t3 = getTickCount("nano");
writeOutput("amp-chain(13): " & numberFormat((t1-t0)/n,"0") & " ns | interp: " & numberFormat((t2-t1)/n,"0") & " ns | &= x11: " & numberFormat((t3-t2)/n,"0") & " ns" & chr(10));
</cfscript>
<cfscript>
function fnAppend(n) { var s = ""; for (var i=1;i<=n;i++) { s = ""; s &= "ALTER TABLE `"; s &= "tbl"; s &= "` MODIFY COLUMN `"; s &= "col"; s &= "` "; s &= "VARCHAR(255)"; s &= " NOT NULL"; s &= " DEFAULT ''"; s &= " COMMENT '"; s &= i; s &= "';"; } return s; }
function fnChain(n) { var s = ""; var tbl="tbl"; var col="col"; for (var i=1;i<=n;i++) { s = "ALTER TABLE `" & tbl & "` MODIFY COLUMN `" & col & "` " & "VARCHAR(255)" & " NOT NULL" & " DEFAULT ''" & " COMMENT '" & i & "'" & ", ADD INDEX `ix_" & col & "` (`" & col & "`)" & ";"; } return s; }
t4 = getTickCount("nano"); fnAppend(n); t5 = getTickCount("nano"); fnChain(n); t6 = getTickCount("nano");
writeOutput("in-function &= x11: " & numberFormat((t5-t4)/n,"0") & " ns | in-function amp-chain(13): " & numberFormat((t6-t5)/n,"0") & " ns" & chr(10));
</cfscript>
