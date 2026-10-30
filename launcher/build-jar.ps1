param(
    [Parameter(Mandatory = $true)][string]$NativeDirectory,
    [Parameter(Mandatory = $true)][string]$OutputJar,
    [switch]$AllowPartialNatives
)
$ErrorActionPreference = 'Stop'
$nativeRoot = (Resolve-Path -LiteralPath $NativeDirectory).Path
$outputPath = [System.IO.Path]::GetFullPath($OutputJar)
$stage = Join-Path $PSScriptRoot ('build-' + [guid]::NewGuid().ToString('N'))
$classes = Join-Path $stage 'classes'
$natives = Join-Path $classes 'natives'
New-Item -ItemType Directory -Path $natives -Force | Out-Null
try {
    & javac --release 25 -d $classes (Join-Path $PSScriptRoot 'src/main/java/io/sryza/Bootstrap.java')
    if ($LASTEXITCODE -ne 0) { throw "javac failed with exit code $LASTEXITCODE" }
    $found = 0
    foreach ($name in @('sryzamc-linux-x86_64', 'sryzamc-linux-aarch64', 'sryzamc-windows-x86_64.exe')) {
        $source = Join-Path $nativeRoot $name
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            if ($AllowPartialNatives) { continue }
            throw "Missing native engine: $source"
        }
        Copy-Item -LiteralPath $source -Destination (Join-Path $natives $name)
        $hash = (Get-FileHash -LiteralPath (Join-Path $natives $name) -Algorithm SHA256).Hash.ToLowerInvariant()
        [System.IO.File]::WriteAllText((Join-Path $natives ($name + '.sha256')), $hash, [System.Text.Encoding]::ASCII)
        $found++
    }
    if ($found -eq 0) { throw "No native engines found in $nativeRoot" }
    New-Item -ItemType Directory -Path (Split-Path -Parent $outputPath) -Force | Out-Null
    & jar --create --file $outputPath --main-class io.sryza.Bootstrap -C $classes .
    if ($LASTEXITCODE -ne 0) { throw "jar failed with exit code $LASTEXITCODE" }
    Write-Output "Created $outputPath"
}
finally {
    $resolvedStage = [System.IO.Path]::GetFullPath($stage)
    if (-not $resolvedStage.StartsWith([System.IO.Path]::GetFullPath($PSScriptRoot), [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to clean staging path outside launcher directory: $resolvedStage"
    }
    Remove-Item -LiteralPath $resolvedStage -Recurse -Force -ErrorAction SilentlyContinue
}
