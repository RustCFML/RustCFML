/**
 * A component that writes its static scope from the PSEUDO-CONSTRUCTOR — the
 * component body, outside any `static {}` block.
 *
 * The parser consumed a leading `static` token as a member MODIFIER
 * (`static function` / `static property` / `static { … }`) whatever followed it,
 * so `static.X = v` at body level lost its scope reference and the remaining
 * `.X = v` parsed as junk: the write vanished with no error and the read came
 * back null (GH #353). `static` is only a modifier when what follows is not a
 * scope reference.
 *
 * Both spellings are covered, and a static modifier in every position it can
 * legally appear, so the guard cannot be widened into eating a real modifier.
 */
component {
    static { static.FromBlock = "block"; }
    static.FromCtor = "ctor";
    static["FromBracket"] = "bracket";

    static function sf() { return "sf"; }
    static string function stf() { return "stf"; }
    public static function psf() { return "psf"; }

    function readCtor()    { return static.FromCtor    ?: "(null)"; }
    function readBracket() { return static.FromBracket ?: "(null)"; }
    function readBlock()   { return static.FromBlock   ?: "(null)"; }
    function callModifiers() { return sf() & "/" & stf() & "/" & psf(); }
}
