fn register() {
    [#{ kind: "column", id: "big", title: "Big", function: "big" }]
}

fn big(ctx) {
    let text = "";
    for i in 0..5000 {
        text += "x";
    }
    [#{ handle: 1, text: text }]
}
