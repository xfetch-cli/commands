<h1 align="center">xfetch commands</h1>

<p>
  Official command addons for the <strong><a href="https://github.com/xfetch-cli/xfetch">xfetch</a></strong>
  ecosystem. Command addons expose extra CLI commands that are installed and
  run through xfetch.
</p>

<h2>Usage</h2>

<pre><code class="language-bash">xfetch command install github
xfetch run github -- rust-lang/rust
xfetch run github -- https://github.com/rust-lang/rust
</code></pre>

<p>
  Command output is plain text with one field per line. Use
  <code>xfetch run github --render -- owner/repo</code> to pass the lines
  through the core layout and logo.
</p>

<h2>Catalog</h2>

<p>
  <code>index.json</code> is the command catalog consumed by
  <code>xfetch command search</code> and <code>xfetch command install</code>.
  Each entry points to a source directory with an <code>xfetch-command.json</code>
  manifest and a WebAssembly build.
</p>

<h2>Add a command</h2>

<ol>
  <li>Create <code>commands/&lt;name&gt;</code> with an
  <code>xfetch-command.json</code> manifest, <code>kind: "command"</code>,
  <code>runtime: "core"</code> and an HTTP/exec/fs allowlist if needed.</li>
  <li>Implement the JSON request / plain-text response protocol. Rust guests can
  use <code>xfetch-guest-api</code>; the API repository also publishes
  <code>xfetch-command-api</code> once released.</li>
  <li>Build to <code>wasm32-wasip1</code>, add the catalog entry and open a PR.</li>
</ol>

<h2>Security</h2>

<ul>
  <li>Wasm commands run under the xfetch wasm policy; capabilities are deny-by-default.</li>
  <li>Native commands are unsandboxed and trusted code.</li>
  <li>Never write logs to stdout; use stderr for diagnostics.</li>
</ul>
