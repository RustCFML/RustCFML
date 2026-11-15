<!--- Included by test_transaction_lucee_semantics.cfm (case P6): a bodyless
      <cftransaction action="rollback" /> inside a nested tag block rolls back
      the whole transaction, which stays open. Lucee 7.1.2: [3,4]. --->
<cftransaction><cfquery datasource="#txnDs#">insert into #txnTable# values (1)</cfquery><cftransaction><cfquery datasource="#txnDs#">insert into #txnTable# values (2)</cfquery><cftransaction action="rollback" /><cfquery datasource="#txnDs#">insert into #txnTable# values (3)</cfquery></cftransaction><cfquery datasource="#txnDs#">insert into #txnTable# values (4)</cfquery></cftransaction>
