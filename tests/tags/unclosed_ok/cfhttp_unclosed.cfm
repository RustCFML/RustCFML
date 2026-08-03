<!--- Fixture: <cfhttp> with no closing tag COMPILES on Lucee — it runs attribute-only
     and the body stays page content. The url is unreachable on purpose; cfhttp
     without throwOnError does not raise for a failed connection. --->
<cfhttp url="http://127.0.0.1:1/none" timeout="1">body
