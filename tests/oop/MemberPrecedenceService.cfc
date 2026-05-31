component {
    function delete(required string id) {
        return "deleted:" & arguments.id;
    }

    function count() {
        return "component-count";
    }
}
