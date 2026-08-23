<!--- Included at page (non-function) level: the caller has no function-local
      scope, so page `variables` must NOT appear as `local.*` here. --->
<!--- isDefined() first: on Lucee a page template has NO local scope at all, so
      naming `local` directly throws before the question can be answered. This
      engine does expose one (a divergence, tracked separately) — the leak test
      is the same either way: page vars must not be visible through it. --->
<cfset request._incLocalPageLeak = isDefined("local")
        AND structKeyExists(local, "_incLocalPageVar")>
