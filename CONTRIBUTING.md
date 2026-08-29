# Contributing to autotun

Thank you for your interest in contributing to `autotun`!

## Getting Started

1. Fork the repository on GitHub.
2. Clone your fork locally:
   ```sh
   git clone https://github.com/<your-username>/autotun.git
   cd autotun
   ```
3. Create a feature branch:
   ```sh
   git checkout -b feat/my-feature
   ```

## Development & Testing

Make sure you have Rust (stable) and OpenSSH installed.

Run the test suite and quality checks before submitting a pull request:

```sh
# Check formatting
cargo fmt --check

# Run unit and integration tests
cargo test --locked

# Run Clippy lints
cargo clippy --locked --all-targets -- -D warnings

# Run installer test script
tests/install.sh
```

### Desktop GUI Dependencies

If working on GUI features (`--features gui`), ensure required system libraries are installed (Debian/Ubuntu):

```sh
sudo apt-get install --yes \
  libwayland-dev libxkbcommon-dev libx11-dev \
  libxcursor-dev libxi-dev libxrandr-dev libxss-dev \
  libgl1-mesa-dev libegl1-mesa-dev pkg-config
```

Verify the GUI builds:

```sh
cargo check --locked --features gui --bins
```

## Pull Request Guidelines

- Keep pull requests focused on a single change or feature.
- Use clear commit messages following Conventional Commits format (e.g., `feat:`, `fix:`, `docs:`, `refactor:`).
- Ensure all CI checks and tests pass locally before opening a pull request.
