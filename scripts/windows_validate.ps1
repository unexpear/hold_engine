param(
    [switch]$RunTests
)

$ErrorActionPreference = 'Stop'

Write-Host '[hold_engine] Windows validation starting...'
Write-Host '-> cargo check'
cargo check

Write-Host '-> cargo build --example browser_backend'
cargo build --example browser_backend

if ($RunTests) {
    Write-Host '-> cargo test'
    cargo test

    Write-Host '-> cargo test --example browser_backend'
    cargo test --example browser_backend

    if (Get-Command node -ErrorAction SilentlyContinue) {
        Write-Host '-> node --test morphic-web-prototype/src/SceneCodec.test.mjs morphic-web-prototype/src/AssetBrowser.test.mjs'
        node --test morphic-web-prototype/src/SceneCodec.test.mjs morphic-web-prototype/src/AssetBrowser.test.mjs
    } else {
        Write-Host '-> skipping browser prototype tests (node not found)'
    }
} else {
    Write-Host '-> skipping cargo test (pass -RunTests to enable)'
}

Write-Host '[hold_engine] Windows validation complete.'
