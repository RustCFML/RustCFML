<cfcomponent output="false" hint="Same shape with a method named cgi() -- the cgi scope must survive too (url/form/cgi/cookie share one store path).">
    <cffunction name="cgi" returntype="string" output="false">
        <cfreturn "fn-cgi" />
    </cffunction>
</cfcomponent>
