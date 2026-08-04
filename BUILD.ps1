# Silanes 一键构建: Rust 构建与测试 + 安装包
# 用法: .\BUILD.ps1 [-SkipTests]

param(
    [switch]$SkipTests
)

$ErrorActionPreference = "Stop"
$ProjectRoot = $PSScriptRoot
$ISCC = "C:\Program Files\Inno Setup 7\ISCC.exe"

# 1. 读取版本号 (Cargo.toml)
$cargoToml = Get-Content "$ProjectRoot\rust\Cargo.toml" -Raw
$rsVersion = [regex]::Match($cargoToml, '^version = "([^"]+)"', 'Multiline').Groups[1].Value.Trim()
Write-Host "Version (Rust): $rsVersion" -ForegroundColor Cyan

# 2. 构建 Rust 项目 (release)
Write-Host "Building Rust project..." -ForegroundColor Yellow
Push-Location "$ProjectRoot\rust"
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "Rust build failed" }

    # 2.5 运行 Rust 单元测试
    if (-not $SkipTests) {
        Write-Host "Running Rust unit tests..." -ForegroundColor Yellow
        cargo test --release
        if ($LASTEXITCODE -ne 0) { throw "Rust tests failed" }
    }
} finally {
    Pop-Location
}

# 复制 Rust exe 到 publish (先清空, 确保目录里只有最终产物)
$rustPublish = Join-Path $ProjectRoot "rust\publish"
New-Item -ItemType Directory -Force -Path $rustPublish | Out-Null
Get-ChildItem $rustPublish -Force | Remove-Item -Recurse -Force
Copy-Item (Join-Path $ProjectRoot "rust\target\release\silanes64.exe") (Join-Path $rustPublish "silanes64.exe") -Force

# 3. 更新 installer.iss 的版本号和版权年份
Write-Host "Updating installer.iss..." -ForegroundColor Yellow
$year = (Get-Date).Year

$rsIss = Get-Content "$ProjectRoot\rust\installer.iss" -Raw -Encoding UTF8
$rsIss = $rsIss -replace '(?m)^#define MyAppVersion ".*"$', "#define MyAppVersion `"$rsVersion`""
$rsIss = $rsIss -replace '(?m)(?<=^#define MyAppPublisher "Copyright \(C\) )\d{4}', $year
[System.IO.File]::WriteAllText("$ProjectRoot\rust\installer.iss", $rsIss, [System.Text.UTF8Encoding]::new($false))

# 4. 编译安装包
Write-Host "Compiling installer..." -ForegroundColor Yellow
& $ISCC "$ProjectRoot\rust\installer.iss"
if ($LASTEXITCODE -ne 0) { throw "Installer build failed" }

$setupName = "silanes-win-x64-setup-v$rsVersion.exe"
Write-Host "Done: rust\publish\silanes64.exe" -ForegroundColor Green
Write-Host "Done: rust\publish\$setupName" -ForegroundColor Green
