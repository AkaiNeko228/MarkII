# Builds everything and creates the release zips in dist/.
#   powershell -ExecutionPolicy Bypass -File scripts\package.ps1
# Fails if any required file is missing from a zip, so a release can never
# ship without the sensors helper or the PawnIO installer.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$ver = (Select-String -Path "crates\markii-gpui\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
Write-Host "Mark II v$ver"

Write-Host "== sensors helper (.NET)"
dotnet publish sensors -c Release -o target\sensors -v q
if ($LASTEXITCODE) { throw "dotnet publish failed" }

Write-Host "== GPUI"
cargo build --release -p markii-gpui
if ($LASTEXITCODE) { throw "GPUI build failed" }

Write-Host "== Tauri"
cargo build --release -p markii-tauri --features tauri/custom-protocol
if ($LASTEXITCODE) { throw "Tauri build failed" }

$licNote = @"
Componentes de terceiros incluidos no Mark II

PawnIO_setup.exe - instalador oficial do driver PawnIO 2.2.0, sem modificacao.
  Autor: namazso. Licenca: GNU GPL v2 (PawnIO-GPL-2.0.txt).
  Codigo-fonte: https://github.com/namazso/PawnIO
  Instalador original: https://github.com/namazso/PawnIO.Setup/releases/tag/2.2.0

markii-sensors.exe inclui a LibreHardwareMonitorLib 0.9.6.
  Licenca: Mozilla Public License 2.0 (LibreHardwareMonitor-MPL-2.0.txt).
  Codigo-fonte: https://github.com/LibreHardwareMonitor/LibreHardwareMonitor
"@

function Readme($exe, $label) {
@"
Mark II v$ver - versao $label

1. Execute $exe. Mantenha os outros arquivos desta pasta junto dele.
2. O Windows vai pedir permissao de administrador (UAC) para o servico
   de sensores. Com ela o app le CPU (potencia, Vcore), ventoinhas,
   placa-mae e timings da RAM. Se recusar, o app continua funcionando
   com GPU e discos e mostra um aviso com "Tentar com administrador".
3. Na primeira vez, se o driver PawnIO nao estiver instalado, clique em
   "Instalar PawnIO" no aviso. O instalador oficial ja vem nesta pasta
   (PawnIO_setup.exe); nada e baixado da internet.

Sensores: LibreHardwareMonitorLib (MPL-2.0) + driver PawnIO (GPL-2.0), veja a pasta licencas.
O servico de sensores so envia dados para o app; ele nao recebe comandos.
Configuracoes: %APPDATA%\com.akaineko.markii\config.json
"@
}

$builds = @(
    @{ name = "windows-x64";       exe = "MarkII.exe";       src = "target\release\markii.exe";       label = "GPUI (nativa)" },
    @{ name = "tauri-windows-x64"; exe = "MarkII-Tauri.exe"; src = "target\release\markii-tauri.exe"; label = "Tauri (interface web, requer WebView2)" }
)
$required = @("markii-sensors.exe", "PawnIO_setup.exe", "LEIA-ME.txt", "licencas\PawnIO-GPL-2.0.txt", "licencas\LibreHardwareMonitor-MPL-2.0.txt")

foreach ($b in $builds) {
    $dir = "dist\v$ver\$($b.name)"
    if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    New-Item -ItemType Directory -Force "$dir\licencas" | Out-Null
    Copy-Item $b.src "$dir\$($b.exe)"
    Copy-Item "target\sensors\markii-sensors.exe" $dir
    Copy-Item "vendor\pawnio\PawnIO_setup.exe" $dir
    Copy-Item "vendor\pawnio\COPYING.txt" "$dir\licencas\PawnIO-GPL-2.0.txt"
    Copy-Item "vendor\LibreHardwareMonitor-MPL-2.0.txt" "$dir\licencas\LibreHardwareMonitor-MPL-2.0.txt"
    [IO.File]::WriteAllText("$root\$dir\licencas\LEIA-ME-LICENCAS.txt", $licNote)
    [IO.File]::WriteAllText("$root\$dir\LEIA-ME.txt", (Readme $b.exe $b.label))

    $zip = "dist\MarkII-v$ver-$($b.name).zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path "$dir\*" -DestinationPath $zip

    # verify the zip itself, not the folder
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $z = [IO.Compression.ZipFile]::OpenRead("$root\$zip")
    $names = $z.Entries | ForEach-Object { $_.FullName -replace '/', '\' }
    $z.Dispose()
    foreach ($f in @($b.exe) + $required) {
        if ($names -notcontains $f) { throw "$zip is missing $f" }
    }
    $mb = [math]::Round((Get-Item $zip).Length / 1MB, 1)
    Write-Host "OK $zip ($mb MB): $($names -join ', ')"
}
