$ErrorActionPreference = "Stop"

$RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

foreach ($Command in @("cargo", "node", "npm")) {
    if (-not (Get-Command $Command -ErrorAction SilentlyContinue)) {
        throw "Required command '$Command' was not found in PATH."
    }
}

$NodeMajor = [int](& node -p "process.versions.node.split('.')[0]")
if ($NodeMajor -lt 24) {
    throw "Node.js 24 or newer is required; found $(& node --version)."
}

Push-Location $RepositoryRoot
try {
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace --no-fail-fast
    npm --prefix apps/desktop ci
    npm --prefix apps/desktop test
    npm --prefix apps/desktop run build
}
finally {
    Pop-Location
}
