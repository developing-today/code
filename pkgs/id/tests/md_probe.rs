// Probe: markdown -> PM -> markdown -> PM round-trip fixpoint checks.
use id::web::{markdown_to_prosemirror, prosemirror_to_markdown};

fn check(name: &str, md: &str) {
    let pm1 = markdown_to_prosemirror(md);
    let md2 = match prosemirror_to_markdown(&pm1) {
        Ok(s) => s,
        Err(e) => {
            println!("=== {name}: SAVE ERROR {e}\n  in: {md:?}");
            return;
        }
    };
    let pm2 = markdown_to_prosemirror(&md2);
    if pm1 != pm2 {
        println!("=== {name}: NOT A FIXPOINT\n  in : {md:?}\n  out: {md2:?}\n  pm1: {pm1}\n  pm2: {pm2}");
    } else if md2.trim() != md.trim() {
        println!("--- {name}: stable but rewritten\n  in : {md:?}\n  out: {md2:?}");
    } else {
        println!("ok  {name}");
    }
}

#[test]
fn probe_battery() {
    check("link-title", "[a](https://x.test \"T\")\n");
    check("ref-link", "[a][r]\n\n[r]: https://x.test\n");
    check("autolink", "<https://x.test/path>\n");
    check("fence-lang", "```rust\nfn main() {}\n```\n");
    check("fence-tilde", "~~~text\nhi\n~~~\n");
    check("table-align", "| a | b | c |\n|:---|:---:|---:|\n| 1 | 2 | 3 |\n");
    check("task-ordered", "1. [x] done\n2. [ ] todo\n");
    check("setext", "Title\n=====\n\nSub\n-----\n");
    check("escaped-star", "\\*x\\* not italic\n");
    check("entity-amp", "fish &amp; chips\n");
    check("entity-copy", "&copy; 2026\n");
    check("hardbreak-backslash", "a\\\nb\n");
    check("img-title", "![alt](x.png \"T\")\n");
    check("footnote-multipara", "Text[^a].\n\n[^a]: first\n\n    second para\n");
    check("heading-code", "# Hello `code` world\n");
    check("strike-in-link", "[~~old~~ new](https://x.test)\n");
    check("code-span-backtick", "``a `b` c`` done\n");
    check("underscore-midword", "a_b_c and d_e_f\n");
    check("strong-in-link", "[**bold** and *em*](https://x.test)\n");
    check("html-block", "<div>\n\nhi\n\n</div>\n");
    check("hardbreak-spaces", "a  \nb\n");
    check("ol-start3", "3. three\n4. four\n");
    check("empty-link", "[empty]()\n");
    check("img-alt-brackets", "![a [b] c](x.png)\n");
    check("blockquote-list", "> - one\n> - two\n");
    check("nested-list", "- a\n  - b\n    - c\n");
    check("link-parens-dest", "[a](https://x.test/f_(1))\n");
    check("multi-para-quote", "> one\n>\n> two\n");
    check("hr-stars", "***\n");
    check("code-block-in-list", "- item\n\n      code();\n");
    check("link-empty-text", "[](https://x.test)\n");
    check("em-underscore", "_hello_ world\n");
    check("strong-underscore", "__hello__ world\n");
    check("del-word", "~~struck~~\n");
    check("heading-trailing", "## Title ##\n");
    check("tab-list", "- a\n\t- b\n");
    check("checklist-nested", "- [ ] a\n  - [x] b\n");
    check("image-in-link", "[![alt](x.png)](https://x.test)\n");
    check("bare-url", "visit https://x.test/path now\n");
    check("email-autolink", "<a@b.co>\n");
    check("smart-quote-text", "it's a \"test\" -- dash\n");
    check("nbsp-entity", "a&nbsp;b\n");
    check("lt-entity", "1 &lt; 2\n");
    check("code-span-spaces", "` code ` x\n");
    check("link-title-single", "[a](https://x.test 'T')\n");
    check("link-title-parens", "[a](https://x.test (T))\n");
}
