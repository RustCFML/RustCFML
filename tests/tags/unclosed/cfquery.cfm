<!--- Fixture: cfquery with no closing tag — Lucee fails at RUNTIME here (no SQL), not at compile time. --->
<cfquery name="q" datasource="nope">select 1
