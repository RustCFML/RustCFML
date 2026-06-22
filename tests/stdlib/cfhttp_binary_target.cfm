<cfscript>
payload = binaryDecode("00FF10414280", "hex");
cfcontent(type = "application/octet-stream", variable = payload, reset = true);
</cfscript>
