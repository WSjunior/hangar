# Tira o Electron (shell/) das maquinas Windows que ainda o tem: o app nativo ja atende tudo o que so ele
# fazia aqui (hangar-preview e a tela remota do navegador no celular). Roda pelo passo de atualizacao.
# Arquivo so em ASCII: o PowerShell 5.1 le sem BOM como cp1252.
#
# Nunca falha: passo que falha derruba o Atualizar inteiro. Sem o app nativo nao mexe em nada (o Electron
# e a unica janela da maquina). Com o Electron aberto tira so os atalhos; o shell\node_modules fica, porque
# o binario em uso nao se apaga. O codigo de shell/ fica (o hangar-preview importa modulos dele), e o
# %APPDATA%\Electron tambem (o app nativo importa servidores e aparencia de la).
# Uso: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/remover-electron.ps1
$ErrorActionPreference = 'Continue'
$raiz = Split-Path -Parent $PSScriptRoot
$marcaDir = Join-Path $HOME '.hangar\native'
$marca = Join-Path $marcaDir 'electron-removido'
$nativo = Join-Path $env:LOCALAPPDATA 'Programs\Hangar\Hangar.exe'
$modulos = Join-Path $raiz 'shell\node_modules'

function Marca($texto) {
    New-Item -ItemType Directory -Force -Path $marcaDir | Out-Null
    [IO.File]::WriteAllText($marca, $texto)
}

if (-not (Test-Path -LiteralPath $nativo)) {
    Write-Host "electron: o app nativo nao esta instalado; o Electron fica"
    Marca 'sem app nativo: nada removido'
    exit 0
}

foreach ($pasta in @([Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('DesktopDirectory'))) {
    if (-not $pasta) { continue }
    $atalho = Join-Path $pasta 'Hangar (Electron).lnk'
    if (Test-Path -LiteralPath $atalho) {
        Remove-Item -LiteralPath $atalho -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $atalho) { Write-Host "electron: nao consegui apagar $atalho" }
        else { Write-Host "electron: atalho removido: $atalho" }
    }
}

# Pelo fim do caminho, nao pela raiz: o mesmo checkout pode aparecer com dois nomes (unidade mapeada e UNC).
$aberto = @(Get-Process -Name electron -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and $_.Path -like '*\shell\node_modules\electron\dist\electron.exe' })
if (-not (Test-Path -LiteralPath $modulos)) {
    Marca 'atalhos removidos; shell\node_modules ja nao existia'
} elseif ($aberto.Count -gt 0) {
    Write-Host "electron: o Hangar (Electron) esta aberto; os atalhos sairam, e o shell\node_modules fica ate a pasta ser apagada com ele fechado"
    Marca 'atalhos removidos; shell\node_modules mantido porque o Electron estava aberto'
} else {
    Remove-Item -LiteralPath $modulos -Recurse -Force -ErrorAction SilentlyContinue
    # O Remove-Item do 5.1 falha em caminho longo e em link de pasta; o rd do cmd passa nos dois.
    if (Test-Path -LiteralPath $modulos) { cmd /c "rd /s /q `"$modulos`"" 2>$null | Out-Null }
    if (Test-Path -LiteralPath $modulos) {
        Write-Host "electron: nao consegui apagar $modulos"
        Marca 'atalhos removidos; shell\node_modules nao saiu'
    } else {
        Write-Host "electron: $modulos removido"
        Marca 'atalhos e shell\node_modules removidos'
    }
}
exit 0
