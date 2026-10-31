# By-name connect/emit behind a default-off feature

Dynamic (by-name) connections and invocations exist — tooling and debug consoles need them — but live behind the default-off cargo feature `dynamic-invoke`, funneled through a single entry point with signature validation, `Result` returns, and logged lookups. Safety-critical builds therefore cannot contain the by-name path at all: "stop at level 3" is a build flag, not discipline.
