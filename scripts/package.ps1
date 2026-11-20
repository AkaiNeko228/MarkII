# Builds everything and creates the release zips in dist/.
#   powershell -ExecutionPolicy Bypass -File scripts\package.ps1
# Fails if any required file is missing from a zip, so a release can never
# ship without the sensors helper or the PawnIO installer.
# Only the GPUI app by default; -Tauri also builds and zips the Tauri version.
param([switch]$Tauri)
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

if ($Tauri) {
    Write-Host "== Tauri"
    cargo build --release -p markii-tauri --features tauri/custom-protocol
    if ($LASTEXITCODE) { throw "Tauri build failed" }
}

Write-Host "== OpenRGB 1.1 (cache em vendor\openrgb, conferido por SHA-256)"
$orgbZip = "vendor\openrgb\OpenRGB_1.1_Windows_64_8e16de2.zip"
$orgbSha = "63291726c6fc84eb479ef2ee752e97e1187f6ce9c49fd27eee40ded588a8172a"
$orgbDir = "vendor\openrgb\OpenRGB Windows 64-bit"
New-Item -ItemType Directory -Force "vendor\openrgb" | Out-Null
if (-not (Test-Path $orgbZip)) {
    Invoke-WebRequest "https://codeberg.org/OpenRGB/OpenRGB/releases/download/release_1.1/OpenRGB_1.1_Windows_64_8e16de2.zip" -OutFile $orgbZip
}
if ((Get-FileHash $orgbZip -Algorithm SHA256).Hash.ToLower() -ne $orgbSha) { throw "OpenRGB zip com hash errado" }
if (-not (Test-Path "$orgbDir\OpenRGB.exe")) { Expand-Archive $orgbZip -DestinationPath "vendor\openrgb" -Force }

$licNote = @"
Componentes de terceiros incluidos no Mark II

PawnIO_setup.exe - instalador oficial do driver PawnIO 2.2.0, sem modificacao.
  Autor: namazso. Licenca: GNU GPL v2 (PawnIO-GPL-2.0.txt).
  Codigo-fonte: https://github.com/namazso/PawnIO
  Instalador original: https://github.com/namazso/PawnIO.Setup/releases/tag/2.2.0

markii-sensors.exe inclui a LibreHardwareMonitorLib 0.9.6.
  Licenca: Mozilla Public License 2.0 (LibreHardwareMonitor-MPL-2.0.txt).
  Codigo-fonte: https://github.com/LibreHardwareMonitor/LibreHardwareMonitor

Logos de marcas (Intel, AMD, NVIDIA, ASUS, MSI, Corsair, Kingston...) - Simple Icons,
  dominio publico (CC0 1.0, Simple-Icons-CC0.txt). As marcas pertencem aos seus donos
  e sao usadas so para identificar o hardware deste PC. https://simpleicons.org
"@
$licOpenRgb = @"

openrgb\ (so na versao GPUI) - OpenRGB 1.1, binarios oficiais sem modificacao.
  Licenca: GNU GPL v2 (OpenRGB-GPL-2.0.txt). Usa Qt 6 (LGPL v3), hidapi, libusb e mbedtls.
  Codigo-fonte: https://codeberg.org/OpenRGB/OpenRGB (tag release_1.1)
  Binario original: https://codeberg.org/OpenRGB/OpenRGB/releases/tag/release_1.1
  O Mark II roda o OpenRGB como programa separado e conversa com ele pelo SDK local
  (127.0.0.1:6742) para controlar o RGB de memoria, placa de video, ventoinhas e coolers.
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

$rgbNote = @"

RGB (botao "RGB" no topo): controla as luzes de memoria, placa de video, placa-mae,
ventoinhas e coolers com o OpenRGB que vem na pasta openrgb. Memoria, placa de video
e placa-mae precisam do driver PawnIO e de administrador. "Reaplicar ao ligar o PC"
cria a tarefa "Mark II RGB" no Agendador de Tarefas, que aplica as cores salvas no
login sem abrir o Mark II (para remover, desmarque a opcao).
"@

$licOpenVr = @"

openvr_api.dll (so na versao GPUI) - OpenVR SDK 2.15.6 da Valve, binario oficial sem modificacao.
  Licenca: BSD 3-Clause (OpenVR-BSD-3.txt). https://github.com/ValveSoftware/openvr
"@

$vrNote = @"

VR (botao "VR" no topo ou Configuracoes > Realidade virtual): mostra um painel do
Mark II no pulso esquerdo dentro do SteamVR (Quest via Link/Air Link/Virtual Desktop,
Index, Vive...). Desligado por padrao. Nao abre o SteamVR sozinho e fecha junto com
o Mark II; desligado, nao roda nada.
"@

$builds = @(
    @{ name = "windows-x64";       exe = "MarkII.exe";       src = "target\release\markii.exe";       label = "GPUI (nativa)"; rgb = $true },
    @{ name = "tauri-windows-x64"; exe = "MarkII-Tauri.exe"; src = "target\release\markii-tauri.exe"; label = "Tauri (interface web, requer WebView2)"; rgb = $false }
) | Where-Object { $Tauri -or -not $_.name.StartsWith("tauri") }
$required = @("markii-sensors.exe", "PawnIO_setup.exe", "LEIA-ME.txt", "licencas\PawnIO-GPL-2.0.txt", "licencas\LibreHardwareMonitor-MPL-2.0.txt", "licencas\Simple-Icons-CC0.txt")

foreach ($b in $builds) {
    $dir = "dist\v$ver\$($b.name)"
    if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    New-Item -ItemType Directory -Force "$dir\licencas" | Out-Null
    Copy-Item $b.src "$dir\$($b.exe)"
    Copy-Item "target\sensors\markii-sensors.exe" $dir
    Copy-Item "vendor\pawnio\PawnIO_setup.exe" $dir
    Copy-Item "vendor\pawnio\COPYING.txt" "$dir\licencas\PawnIO-GPL-2.0.txt"
    Copy-Item "vendor\LibreHardwareMonitor-MPL-2.0.txt" "$dir\licencas\LibreHardwareMonitor-MPL-2.0.txt"
    Copy-Item "crates\markii-core\assets\logos\LICENSE-CC0.md" "$dir\licencas\Simple-Icons-CC0.txt"
    if ($b.rgb) {
        Copy-Item $orgbDir "$dir\openrgb" -Recurse
        Copy-Item "$orgbDir\LICENSE" "$dir\licencas\OpenRGB-GPL-2.0.txt" -ErrorAction SilentlyContinue
        if (-not (Test-Path "$dir\licencas\OpenRGB-GPL-2.0.txt")) { Copy-Item "vendor\pawnio\COPYING.txt" "$dir\licencas\OpenRGB-GPL-2.0.txt" }
        # SteamVR overlay: official OpenVR runtime loader (BSD-3-Clause)
        Copy-Item "vendor\openvr\openvr_api.dll" $dir
        Copy-Item "vendor\openvr\LICENSE" "$dir\licencas\OpenVR-BSD-3.txt"
    }
    [IO.File]::WriteAllText("$root\$dir\licencas\LEIA-ME-LICENCAS.txt", $licNote + $(if ($b.rgb) { $licOpenRgb + $licOpenVr } else { "" }))
    [IO.File]::WriteAllText("$root\$dir\LEIA-ME.txt", (Readme $b.exe $b.label) + $(if ($b.rgb) { $rgbNote + $vrNote } else { "" }))

    $zip = "dist\MarkII-v$ver-$($b.name).zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path "$dir\*" -DestinationPath $zip

    # verify the zip itself, not the folder
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $z = [IO.Compression.ZipFile]::OpenRead("$root\$zip")
    $names = $z.Entries | ForEach-Object { $_.FullName -replace '/', '\' }
    $z.Dispose()
    $need = @($b.exe) + $required
    if ($b.rgb) { $need += @("openrgb\OpenRGB.exe", "openrgb\PawnIOLib.dll", "licencas\OpenRGB-GPL-2.0.txt", "openvr_api.dll", "licencas\OpenVR-BSD-3.txt") }
    foreach ($f in $need) {
        if ($names -notcontains $f) { throw "$zip is missing $f" }
    }
    $mb = [math]::Round((Get-Item $zip).Length / 1MB, 1)
    Write-Host "OK $zip ($mb MB): $($names -join ', ')"
}
