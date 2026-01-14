# Contributing to LocalRouter AI

Thank you for your interest in contributing to LocalRouter AI! This document provides guidelines and instructions for contributing to the project.

## Development Setup

### Prerequisites

- Rust 1.75 or later
- Node.js 18 or later
- Tauri CLI: `cargo install tauri-cli`

### Getting Started

1. Fork the repository
2. Clone your fork:
   ```bash
   git clone https://github.com/yourusername/localrouterai.git
   cd localrouterai
   ```
3. Build the project:
   ```bash
   cargo build
   ```
4. Run in development mode:
   ```bash
   cargo tauri dev
   ```

## Project Structure

- `src-tauri/` - Rust backend code
- `src/` - Frontend code
- `ARCHITECTURE.md` - System architecture and design
- `PROGRESS.md` - Implementation progress tracking

## Development Workflow

### Before Starting

1. Check `PROGRESS.md` to see what features are available for implementation
2. Review `ARCHITECTURE.md` to understand the system design
3. Open an issue or comment on an existing issue to discuss your plans

### Making Changes

1. Create a new branch:
   ```bash
   git checkout -b feature/your-feature-name
   ```
2. Make your changes
3. Write tests for your changes
4. Ensure all tests pass:
   ```bash
   cargo test
   ```
5. Run formatting and linting:
   ```bash
   cargo fmt
   cargo clippy -- -D warnings
   ```
6. Update `PROGRESS.md` to mark completed features

### Committing

- Use clear, descriptive commit messages
- Follow Conventional Commits format:
  - `feat:` for new features
  - `fix:` for bug fixes
  - `docs:` for documentation changes
  - `test:` for test additions/changes
  - `refactor:` for code refactoring
  - `chore:` for maintenance tasks

Example:
```
feat(providers): add Ollama provider implementation

- Implement ModelProvider trait for Ollama
- Add health check support
- Add model listing via /api/tags endpoint
```

### Submitting Pull Requests

1. Push your branch to your fork
2. Create a pull request against the `main` branch
3. Fill out the pull request template
4. Link any related issues
5. Wait for review and address feedback

## Code Style

### Rust

- Follow the official Rust style guide
- Use `rustfmt` for formatting
- Use `clippy` for linting
- Write documentation comments (`///`) for public APIs
- Keep functions small and focused

### Documentation

- Update relevant documentation when making changes
- Add inline comments for complex logic
- Update `ARCHITECTURE.md` for architectural changes
- Update `PROGRESS.md` for completed features

## Testing

### Unit Tests

- Write unit tests for all new functionality
- Place tests in the same file as the code being tested
- Use the `#[cfg(test)]` module convention

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_something() {
        // Test implementation
    }
}
```

### Integration Tests

- Place integration tests in `tests/` directory
- Test complete workflows and interactions between components

### E2E Tests

- Test the full application from the user's perspective
- Verify UI and backend integration

## Code Review Process

1. All pull requests require at least one approval
2. CI checks must pass (build, tests, linting)
3. Address all review comments
4. Keep discussions professional and constructive

## Feature Implementation

When implementing a new feature:

1. Check the feature in `PROGRESS.md`
2. Review the success criteria for the feature
3. Implement the feature according to the architecture in `ARCHITECTURE.md`
4. Write tests that verify all success criteria
5. Update `PROGRESS.md` to mark the feature as completed
6. Update documentation as needed

## Reporting Bugs

- Use the GitHub issue tracker
- Include a clear title and description
- Provide steps to reproduce
- Include system information (OS, Rust version, etc.)
- Attach relevant logs or screenshots

## Feature Requests

- Use the GitHub issue tracker
- Clearly describe the feature and its use case
- Explain why this feature would be valuable
- Be open to discussion and alternative solutions

## Questions?

- Open a GitHub discussion for general questions
- Comment on relevant issues for specific questions
- Check existing documentation and issues first

## License

By contributing to LocalRouter AI, you agree that your contributions will be licensed under the MIT License.

---

Thank you for contributing to LocalRouter AI!
