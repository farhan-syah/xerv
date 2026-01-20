# XERV Documentation Index

This index provides a complete overview of XERV documentation and guides you to the right resources.

## For New Users

Start here if you're new to XERV:

1. **[README](../README.md)** - Project overview and key concepts (5 min read)
2. **[Getting Started](getting-started.md)** - Setup guide with working example (15 min read)
3. **[Quick Reference](quick-reference.md)** - Node types, triggers, API endpoints at a glance (reference)

## Core Documentation

### Architecture & Design
- **[Architecture Deep Dive](architecture.md)** - System design, data plane, execution plane, linker (30 min read)
  - Memory-mapped arena with rkyv serialization
  - Topological scheduler with DAG execution
  - Selector resolution system
  - Write-ahead log (WAL) for crash recovery
  - Sequence diagrams for trace execution

### Node Development
- **[Writing Custom Nodes](nodes.md)** - Build your own node types (30 min read)
  - Node trait and anatomy
  - Simple validation example
  - Context and external services
  - Conditional, merge, fan-out, and loop nodes
  - Testing nodes with FlowRunner
  - WebAssembly (WASM) nodes
  - Node registration and factories
  - Best practices and patterns

### Pipeline Execution
- **[Testing Guide](testing.md)** - Deterministic testing with mock providers (30 min read)
  - FlowRunner setup and configuration
  - Mocking time (fixed and advancing)
  - Mocking HTTP with request assertions
  - Mocking random numbers (seeds, UUIDs)
  - Mocking filesystem and environment
  - Testing complex flows with branching
  - Error case testing
  - Idempotency and snapshot testing
  - Best practices

## Feature Guides

### Triggers
- **[Triggers](triggers.md)** - Event sources that initiate pipeline execution (20 min read)
  - Webhook trigger (HTTP POST)
  - Cron trigger (scheduled execution)
  - Filesystem trigger (file events)
  - Queue trigger (in-memory messaging)
  - Kafka trigger (distributed events)
  - Memory & manual triggers (testing)
  - Custom trigger implementation
  - Trigger lifecycle and management
  - Testing with triggers
  - Trigger patterns

### Human-in-the-Loop Workflows
- **[Suspension System](suspension-system.md)** - Approval workflows and manual intervention (25 min read)
  - WaitNode for pausing execution
  - Suspension states and lifecycle
  - Querying and resuming suspended traces
  - Timeout handling strategies
  - Custom suspension-aware nodes
  - Testing suspension workflows
  - Persistence implementation
  - Common patterns (escalation, webhooks, auto-approval)
  - Monitoring suspensions

### REST API
- **[REST API Reference](api.md)** - Complete API documentation (20 min read)
  - Health checks
  - Pipeline management endpoints
  - Trace querying and inspection
  - Log streaming (SSE)
  - Suspension management
  - Trigger management
  - Error responses and codes
  - 15+ curl examples
  - Rate limiting configuration
  - Authentication recommendations

## Quick Lookup

### Node Types
See [Quick Reference](quick-reference.md#node-types) for a table of all 9 standard nodes.

### Trigger Types
See [Quick Reference](quick-reference.md#trigger-types) for a table of all 7 trigger types.

### REST Endpoints
See [Quick Reference](quick-reference.md#rest-api-endpoints) for a table of all endpoints.

### Cron Expressions
See [Quick Reference](quick-reference.md#cron-expression-cheat-sheet) for cron syntax reference.

### Rust Code Patterns
See [Quick Reference](quick-reference.md#rust-code-patterns) for common code patterns.

## Learning Path by Role

### Application Developer
Want to integrate XERV into your application?

1. [Getting Started](getting-started.md) - Embedded usage section
2. [REST API Reference](api.md) - For calling the API
3. [Testing Guide](testing.md) - For testing flows

### DevOps/Operations
Want to deploy and manage XERV?

1. [Getting Started](getting-started.md) - Server setup section
2. [Quick Reference](quick-reference.md) - Configuration quick lookup
3. [REST API Reference](api.md) - For monitoring and management
4. [Architecture](architecture.md) - For understanding deployment requirements

### Workflow Designer
Want to create sophisticated pipelines?

1. [README](../README.md) - Core concepts
2. [Quick Reference](quick-reference.md) - Node and trigger cheat sheets
3. [Triggers](triggers.md) - Available event sources
4. [Suspension System](suspension-system.md) - Approval workflows
5. [Architecture](architecture.md) - Understanding execution model

### Node Developer
Want to extend XERV with custom nodes?

1. [Quick Reference](quick-reference.md) - Existing node patterns
2. [Writing Custom Nodes](nodes.md) - Full guide
3. [Testing Guide](testing.md) - Testing your nodes
4. [Architecture](architecture.md) - Understanding data flow

### Framework Developer
Want to contribute to XERV core?

1. [Architecture](architecture.md) - Deep understanding required
2. [Writing Custom Nodes](nodes.md) - Node trait design
3. Build and run tests: `cargo test --all`

## Problem Solving

### "My pipeline won't start"
- Check YAML syntax: [Getting Started](getting-started.md#define-the-flow-yaml)
- See trigger config: [Triggers](triggers.md)
- Troubleshooting: [Getting Started](getting-started.md#common-issues)

### "My trace is hanging"
- Check suspensions: [Suspension System](suspension-system.md#querying-suspended-traces)
- View trace state: [REST API](api.md#get-specific-trace)
- Stream logs: [REST API](api.md#stream-trace-execution-logs)

### "I want to add approvals"
- Read: [Suspension System](suspension-system.md)
- Example: See approval patterns section
- Test: [Testing Guide](testing.md#testing-suspended-traces)

### "I want to process Kafka events"
- Read: [Triggers](triggers.md#kafka-trigger)
- Configure: YAML config section
- Test: [Testing Guide](testing.md#testing-with-triggers)

### "I want custom business logic"
- Read: [Writing Custom Nodes](nodes.md)
- Follow: Simple example section
- Test: [Testing Guide](testing.md#testing-your-node)

### "I want to understand performance"
- Read: [Architecture](architecture.md#performance-characteristics)
- Tips: [Quick Reference](quick-reference.md#performance-tips)
- Monitor: [REST API](api.md) metrics endpoints

## Document Statistics

| Document | Size | Topics |
|----------|------|--------|
| README | 368 lines | Overview, quick start, core concepts |
| Getting Started | 320 lines | Setup, examples, configuration, troubleshooting |
| Quick Reference | 280 lines | Cheat sheets, tables, code patterns |
| Architecture | 706 lines | Deep dive, diagrams, implementation details |
| Triggers | 370 lines | 7 trigger types, patterns, custom implementation |
| Suspension System | 440 lines | Human-in-the-loop, patterns, persistence |
| REST API | 300 lines | 10+ endpoints, curl examples, error codes |
| Writing Nodes | 540 lines | Node development, examples, patterns |
| Testing | 660 lines | Framework, mocks, patterns, best practices |
| **Total** | **~4000 lines** | **Complete ecosystem documentation** |

## How Documentation is Organized

```
xerv/
├── README.md                      # Project overview
└── docs/
    ├── INDEX.md                   # This file
    ├── getting-started.md         # Setup and first example
    ├── quick-reference.md         # Lookup tables and cheat sheets
    ├── architecture.md            # Deep technical details
    ├── triggers.md                # Event source documentation
    ├── suspension-system.md        # Human-in-the-loop workflows
    ├── api.md                     # REST API reference
    ├── nodes.md                   # Custom node development
    └── testing.md                 # Testing guide
```

## Contributing to Documentation

To improve XERV documentation:

1. **For typos or clarifications** - Edit the relevant `.md` file
2. **For new examples** - Add to the appropriate document
3. **For new features** - Create a new guide following the structure above
4. **For code examples** - Ensure they compile with the current codebase
5. **For diagrams** - Use mermaid syntax for consistency

Documentation should:
- Be accurate and reflect actual implementation
- Include working code examples
- Be organized for quick navigation
- Serve both beginners and experienced users
- Provide troubleshooting guidance

## External Resources

- **Repository** - https://github.com/ml-rust/xerv
- **Crate** - https://crates.io/crates/xerv-core
- **Docs.rs** - https://docs.rs/xerv-core

## Version

This documentation is for XERV 0.1.0 (Latest: January 2025)

Last Updated: 2026-01-20
