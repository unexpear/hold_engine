param(
    [switch]$RunTests
)

$ErrorActionPreference = 'Stop'

Write-Host '[hold_engine] Windows validation starting...'
Write-Host '-> cargo check'
cargo check

if ($RunTests) {
    Write-Host '-> cargo test'
    cargo test
} else {
    Write-Host '-> skipping cargo test (pass -RunTests to enable)'
}

Write-Host '[hold_engine] Windows validation complete.'
