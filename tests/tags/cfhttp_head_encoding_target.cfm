<!--- Target for test_cfhttp_head_content_encoding.cfm: answers with the status and Content-Encoding the URL asks
      for, and no body, as a server answers a HEAD for a resource it would compress.
        ?encoding=gzip|br|deflate   the Content-Encoding to name (absent or empty: none)
        ?status=204|304|404         the status to answer with (default 200) --->
<cfset encoding = lCase(trim(url.encoding ?: "")) />
<cfset status = val(url.status ?: 200) />
<cfif status EQ 404>
    <cfheader statuscode="404" statustext="Not Found" />
<cfelseif status EQ 204>
    <cfheader statuscode="204" statustext="No Content" />
<cfelseif status EQ 304>
    <cfheader statuscode="304" statustext="Not Modified" />
</cfif>
<cfif listFind("gzip,br,deflate", encoding)>
    <cfheader name="Content-Encoding" value="#encoding#" />
</cfif>
<cfheader name="X-Head-Target" value="reached" />
<cfcontent type="text/plain" reset="true" />
