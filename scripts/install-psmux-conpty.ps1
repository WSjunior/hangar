# Instala o psmux com console proprio (release `psmux-latest`) em Windows abaixo do build 22523, onde
# o conhost do sistema impede o Claude Code de ligar o mouse e a roda nao rola (psmux/psmux#597).
# Grava PSMUX_CONPTY_DIR e poe a pasta na frente do PATH do usuario. Build 22523+ sai 0 sem tocar em
# nada. Quem reinicia o servidor do psmux e o install.ps1, que pergunta antes.
# Arquivo so em ASCII: o PowerShell 5.1 le sem BOM como cp1252.
# Uso: powershell -ExecutionPolicy Bypass -File scripts/install-psmux-conpty.ps1
$ErrorActionPreference = 'Stop'

$release = if ($env:HANGAR_PSMUX_URL) { $env:HANGAR_PSMUX_URL } else { 'https://github.com/jeffer1312/hangar/releases/download/psmux-latest' }
$destDir = Join-Path $HOME '.hangar\psmux'
$marca = Join-Path $destDir 'release.json'
$pacote = 'psmux-conpty-windows-x86_64.zip'
$arquivos = 'psmux.exe', 'pmux.exe', 'tmux.exe', 'conpty.dll', 'OpenConsole.exe'

$build = [int](Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').CurrentBuildNumber
if ($build -ge 22523) { Write-Host "psmux: Windows build $build ja repassa o mouse; nada a fazer"; exit 0 }
$arq = "$env:PROCESSOR_ARCHITEW6432$env:PROCESSOR_ARCHITECTURE"
if ($arq -notmatch 'AMD64') { Write-Host "psmux: sem build para Windows $arq; a roda do mouse segue sem funcionar no Claude"; exit 0 }

function Sha-Do-Manifesto($texto) {
    if ("$texto" -match ('"' + [regex]::Escape($pacote) + '":\s*"([0-9a-f]{64})"')) { $Matches[1] } else { '' }
}

[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$ProgressPreference = 'SilentlyContinue'
try {
    $bruto = (Invoke-WebRequest -UseBasicParsing -TimeoutSec 30 -Uri "$release/psmux-latest.json").Content
} catch { Write-Host "psmux: nao consegui ler a release em $release"; exit 1 }
if ($bruto -is [byte[]]) { $bruto = [Text.Encoding]::UTF8.GetString($bruto) }
$sha = Sha-Do-Manifesto $bruto
if (-not $sha) { Write-Host "psmux: $pacote nao esta no manifesto da release"; exit 1 }

$instalado = (Test-Path $marca) -and (Sha-Do-Manifesto ([IO.File]::ReadAllText($marca))) -eq $sha -and
             -not ($arquivos | Where-Object { -not (Test-Path (Join-Path $destDir $_)) })
if ($instalado) {
    Write-Host "psmux: build da release ja em $destDir"
} else {
    $tmp = Join-Path $env:TEMP ('hangar-psmux.' + [IO.Path]::GetRandomFileName())
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    try {
        $zip = Join-Path $tmp $pacote
        try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 300 -Uri "$release/$pacote" -OutFile $zip | Out-Null }
        catch { Write-Host "psmux: download de $pacote falhou"; exit 1 }
        if ((Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLower() -ne $sha) {
            Write-Host "psmux: sha256 de $pacote nao confere com a release; nada foi instalado"; exit 1
        }
        Expand-Archive -Path $zip -DestinationPath (Join-Path $tmp 'x') -Force
        New-Item -ItemType Directory -Force -Path $destDir | Out-Null
        Get-ChildItem -Path $destDir -Filter '*.old*' -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
        foreach ($f in Get-ChildItem (Join-Path $tmp 'x') -File) {
            $alvo = Join-Path $destDir $f.Name
            # Imagem de processo vivo nao se sobrescreve, mas se renomeia.
            if (Test-Path $alvo) {
                try { Remove-Item $alvo -Force } catch { Rename-Item $alvo ("$alvo.old-" + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()) }
            }
            Copy-Item $f.FullName $alvo -Force
        }
    } finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Set-ItemProperty -Path 'HKCU:\Environment' -Name 'PSMUX_CONPTY_DIR' -Value $destDir -Type String
# Valor cru, sem expandir: regravar expandido mataria os %VAR% do PATH. Nossa pasta vai primeiro para
# vencer o tmux.exe do winget, que tambem mora no PATH do usuario.
$chave = Get-Item 'HKCU:\Environment'
$path = [string]$chave.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
$resto = @($path -split ';' | Where-Object { $_ -and $_.TrimEnd('\') -ne $destDir.TrimEnd('\') })
$novo = (@($destDir) + $resto) -join ';'
if ($novo -ne $path) { Set-ItemProperty -Path 'HKCU:\Environment' -Name 'Path' -Value $novo -Type ExpandString }
if (-not ('Hangar.AmbientePsmux' -as [type])) {
    Add-Type -Namespace Hangar -Name AmbientePsmux -MemberDefinition @'
[DllImport("user32.dll", SetLastError=true, CharSet=CharSet.Auto)]
public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);
'@
}
$r = [UIntPtr]::Zero
[Hangar.AmbientePsmux]::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$r) | Out-Null

# A marca so existe depois de arquivos, variavel e PATH no lugar.
[IO.File]::WriteAllText($marca, $bruto)
Write-Host "psmux: console proprio em $destDir (PSMUX_CONPTY_DIR e PATH do usuario gravados)"
