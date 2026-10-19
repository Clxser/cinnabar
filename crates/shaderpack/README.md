# Shader-pack loading and configuration

`PackSource` loads text and bounded binary assets from a folder, ZIP archive or memory.
`ShaderPack` discovers options, dimension mappings and shader programs, and parses
`shaders.properties` under a caller-supplied `Environment` and option values.
Program enable rules can be evaluated separately; custom expressions are parsed only.
See the public API documentation for supported metadata and discovery rules.

Shader translation, render-host integration and in-game acceptance remain incomplete.
Opening or inspecting a pack does not activate it in the client.

```sh
cargo test --locked -p shaderpack
```
