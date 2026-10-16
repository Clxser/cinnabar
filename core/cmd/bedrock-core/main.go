// Command bedrock-core is the core process the client starts beside itself.
package main

import "github.com/hashimthearab/rust-mcbe/core/coreapp"

// The build stamps these with -ldflags "-X main.version=... -X main.commit=...".
var (
	version = "unknown"
	commit  = "unknown"
)

func main() {
	coreapp.Version, coreapp.Commit = version, commit
	coreapp.Main(coreapp.Options{})
}
