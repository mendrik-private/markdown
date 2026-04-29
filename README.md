# mdtui

`mdtui` is a keyboard-first terminal Markdown editor with semantic Markdown rendering, table editing, TOC/outline navigation, inline styling, and optional Kitty graphics headlines.

## Install

### Download a release package

Download the latest Debian package from GitHub Releases and install it:

```sh
curl -L -o mdtui.deb https://github.com/mendrik/markdown/releases/latest/download/mdtui.deb
sudo apt install ./mdtui.deb
```

Then open a Markdown file:

```sh
mdtui README.md
```

### Install directly from GitHub with Cargo

```sh
cargo install --git https://github.com/mendrik/markdown.git --package mdtui-tui --bin mdtui
```

### Build from source

```sh
git clone https://github.com/mendrik/markdown.git
cd markdown
cargo build --release --bin mdtui
./target/release/mdtui README.md
```

### Build a Debian package locally

```sh
cargo install cargo-deb
cargo deb -p mdtui-tui
sudo apt install ./target/debian/*.deb
```

## Usage

```sh
mdtui path/to/document.md
```

Useful shortcuts:

| Shortcut | Action |
| --- | --- |
| `Ctrl-S` | Save |
| `Ctrl-Q` | Quit |
| `Ctrl-H` | Help |
| `Shift+Arrow` | Select text |
| `Ctrl-L` | Edit the link under the cursor |
| `Ctrl-C` | Copy selection |
| `Esc` | Close popups |

Kitty-compatible terminals can render H1/H2 headings as graphics. Other terminals fall back to regular text rendering.

## Development

```sh
cargo fmt --all
cargo test --workspace
```

## License

MIT. See [LICENSE](LICENSE).
