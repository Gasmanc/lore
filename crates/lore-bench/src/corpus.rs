//! Synthetic documentation corpus for retrieval benchmarks.
//!
//! Twenty distinct Markdown files covering different technical topics ensure
//! each query has a clear, unique correct answer.

/// Each entry is `(filename, markdown_content)`.
pub const DOCS: &[(&str, &str)] = &[
    ("getting-started.md", include_str!("docs/getting-started.md")),
    ("configuration.md",   include_str!("docs/configuration.md")),
    ("authentication.md",  include_str!("docs/authentication.md")),
    ("error-handling.md",  include_str!("docs/error-handling.md")),
    ("caching.md",         include_str!("docs/caching.md")),
    ("database.md",        include_str!("docs/database.md")),
    ("http-client.md",     include_str!("docs/http-client.md")),
    ("file-io.md",         include_str!("docs/file-io.md")),
    ("logging.md",         include_str!("docs/logging.md")),
    ("testing.md",         include_str!("docs/testing.md")),
    ("performance.md",     include_str!("docs/performance.md")),
    ("deployment.md",      include_str!("docs/deployment.md")),
    ("security.md",        include_str!("docs/security.md")),
    ("cli-reference.md",   include_str!("docs/cli-reference.md")),
    ("types.md",           include_str!("docs/types.md")),
    ("concurrency.md",     include_str!("docs/concurrency.md")),
    ("serialization.md",   include_str!("docs/serialization.md")),
    ("networking.md",      include_str!("docs/networking.md")),
    ("plugins.md",         include_str!("docs/plugins.md")),
    ("migration.md",       include_str!("docs/migration.md")),
];

/// Each entry is `(query, expected_doc_filename)`.
///
/// The expected document is the one that should appear at rank 1.
pub const QUERIES: &[(&str, &str)] = &[
    ("how do I install and set up the library for the first time",    "getting-started.md"),
    ("configure timeout and retry settings via environment variables", "configuration.md"),
    ("authenticate requests using an API key or OAuth token",         "authentication.md"),
    ("handle errors and recover from failures gracefully",            "error-handling.md"),
    ("cache responses with a TTL and invalidate stale entries",       "caching.md"),
    ("connect to a relational database and run queries",              "database.md"),
    ("send an HTTP POST request with a JSON body",                    "http-client.md"),
    ("read the contents of a file from the local filesystem",         "file-io.md"),
    ("configure structured logging with different severity levels",   "logging.md"),
    ("write unit tests and assert on expected behaviour",             "testing.md"),
    ("profile the application and find performance bottlenecks",      "performance.md"),
    ("deploy the application inside a Docker container",              "deployment.md"),
    ("enable TLS and manage certificates for secure communication",   "security.md"),
    ("list all available CLI subcommands and their flags",            "cli-reference.md"),
    ("define a generic type parameter with trait bounds",             "types.md"),
    ("spawn concurrent async tasks and wait for all of them",         "concurrency.md"),
    ("serialize a struct to JSON and deserialize it back",            "serialization.md"),
    ("open a TCP socket and send data over the network",              "networking.md"),
    ("create a custom plugin and register it with the hook system",   "plugins.md"),
    ("migrate from version 1 to version 2 breaking changes",          "migration.md"),
];
