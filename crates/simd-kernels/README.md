# CPU integer kernels

Checked, allocation-free slice operations select AVX2, SSE2 or scalar code at
runtime. Packed indices never cross their source words, and padding remains
unchanged. Vector reads stay within complete batches; final writes use bounded
masked stores, temporary vectors or scalar code. Texture operations preserve RGB bytes and
ignore trailing incomplete pixels.

Cinnabar prints `CPU integer kernels: Avx2`, `Sse2` or `Scalar` at startup.
The default enables the best supported backend. To compare the same build with
explicit SIMD disabled, launch it from PowerShell with:

```powershell
$env:CINNABAR_SIMD = '0'
.\target\play\cinnabar.exe
Remove-Item Env:CINNABAR_SIMD
```

Use the same world, settings and camera route for both runs. Compare chunk
loading and meshing time as well as frame-time percentiles; CPU kernel gains
do not guarantee higher FPS when GPU work or frame pacing limits the client.
