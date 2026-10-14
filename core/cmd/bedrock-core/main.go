// Command bedrock-core is the core process the client starts beside itself.
package main

import "github.com/hashimthearab/rust-mcbe/core/coreapp"

func main() {
	coreapp.Main(coreapp.Options{})
}
