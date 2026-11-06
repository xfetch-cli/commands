# github command

Show GitHub repository information as plain text lines.

```bash
xfetch run github -- rust-lang/rust
xfetch run github -- https://github.com/rust-lang/rust
```

Output:

```text
repo: rust-lang/rust
author: rust-lang
languages: Rust, Shell, Makefile, ...
stars: ...
forks: ...
watchers: ...
open issues: ...
license: MIT
updated: ...
url: https://github.com/rust-lang/rust
description: Empowering everyone to build reliable and efficient software.
```

The command reads the JSON command request from stdin and writes text to
stdout. It uses the sandboxed HTTP host call; the manifest allowlists
`https://api.github.com/repos/*` only.
