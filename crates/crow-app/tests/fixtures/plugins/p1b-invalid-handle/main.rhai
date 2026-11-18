fn register() {
    [#{ kind: "column", id: "bad", title: "Bad handle", function: "bad" }]
}

fn bad(ctx) {
    [#{ handle: 99, text: "never accepted" }]
}
