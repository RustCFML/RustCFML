<cfscript>
// Struct member access — the surface v0.91.0 (member ICs) will target.
// Member access dominates real CFML. Baseline must
// stay flat or improve once Boxed ICs land.
obj = { x: 1, y: 2, z: 3, w: 4 };
total = 0;
for (k = 1; k <= 5000000; k++) {
    total = total + obj.x + obj.y * 2 + obj.z - obj.w;
}
writeOutput(total);

</cfscript>
