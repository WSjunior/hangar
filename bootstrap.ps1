# hangar - clonar e instalar numa linha so, no Windows.
#
#   irm https://raw.githubusercontent.com/jeffer1312/hangar/main/bootstrap.ps1 | iex
#
# Destino: $HOME\hangar. Pra mudar, defina a variavel ANTES da linha acima:
#
#   $env:CP_DESTINO = 'D:\hangar'
#   irm https://.../bootstrap.ps1 | iex
#
# O assistente do app nativo roda este arquivo por -File, e ai o param() abaixo recebe as opcoes e
# as repassa ao install.ps1. -Destino e a pasta do Hangar que o app encontrou (sem ele, vale a
# regra acima):
#
#   powershell -ExecutionPolicy Bypass -File bootstrap.ps1 -Destino D:\hangar -App -Tailscale sim
#
# Sob `irm | iex` nao existe argumento: o param() fica todo no padrao. E o script nao sabe o
# proprio caminho ($MyInvocation.MyCommand.Path vem vazio), entao nada aqui depende dele.
#
# Escrito pra Windows PowerShell 5.1 (o que vem no Windows), igual ao install.ps1: nada de
# operador ternario, `??` nem API de .NET Core.
#
# Este arquivo e ASCII puro e SEM BOM: o `irm` do 5.1 nao tira o BOM, e a primeira linha
# viraria "\uFEFF# ..." - que o iex tenta executar como comando.
#
# Instale num disco LOCAL. Numa pasta compartilhada por rede (`\\servidor\...`) o `uv sync` e o
# `npm ci` recriariam `backend\.venv` e `frontend\node_modules` por cima dos da maquina de
# origem - e esses sao dela, nao seus: o venv aponta pro Python daquela maquina e o node_modules
# traz o binario nativo dela. Instalar de uma segunda maquina quebra a instalacao da primeira.
#
# Sem [ValidateSet] no -Tailscale: sob `irm | iex` o param() vira declaracao de variavel, e o
# atributo recusa o valor vazio do padrao ("would no longer be valid"), derrubando o iex inteiro.
param(
    [string]$Destino,
    [switch]$App,
    [string]$Tailscale,
    [switch]$SemNativo,
    [string]$Agentes,
    [switch]$SoChecar,
    [switch]$Sim,
    [switch]$Avancado,
    [switch]$ConsertarRoda
)

$ErrorActionPreference = 'Stop'

$repoUrl = 'https://github.com/jeffer1312/hangar.git'
$ramo    = 'main'

function Titulo($m) { Write-Host "`n$m" -ForegroundColor Cyan }
function Ok($m)     { Write-Host "  ok  $m" -ForegroundColor Green }
function Nota($m)   { Write-Host "      $m" -ForegroundColor DarkGray }
function Erro($m)   { Write-Host "  X   $m" -ForegroundColor Red }
function Tem($cmd)  { return [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }

# O codigo vem antes da mensagem (o app troca o texto pela frase e pelo botao dele), e no -App a
# ultima linha diz ao assistente que parou aqui, antes do install.ps1.
function Stop-Bootstrap($mensagem, $codigo) {
    if ($codigo) { Write-Host "##HANGAR-ERRO## $codigo" }
    Erro $mensagem
    if ($App) { Write-Host '##HANGAR-FIM## falhou' }
    exit 1
}
# 'sem-internet' quando o github.com nao responde; vazio = a falha e de outra coisa. Mesma conta do
# install.ps1, repetida porque aqui o repositorio ainda nao existe.
function Get-NetworkCode {
    $c = New-Object Net.Sockets.TcpClient
    try { if ($c.ConnectAsync('github.com', 443).Wait(10000)) { return '' } } catch { } finally { $c.Dispose() }
    return 'sem-internet'
}

function Atualiza-Path {
    # winget grava o PATH no registro, mas o PowerShell JA ABERTO segue com o antigo -> o git
    # recem-instalado "nao existe". Mesmo truque do install.ps1.
    # Expandir e ACRESCENTAR, nunca substituir: PATH gravado como REG_SZ volta com %SYSTEMROOT%
    # cru, e um %VAR% literal dentro de $env:Path nao resolve nada - substituir o PATH por isso
    # some com o proprio powershell.exe.
    $registro = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
                [Environment]::GetEnvironmentVariable('Path', 'User')
    $tudo = ($env:Path + ';' + [Environment]::ExpandEnvironmentVariables($registro)) -split ';' |
            Where-Object { $_ } | Select-Object -Unique
    $env:Path = $tudo -join ';'
}

if ($Tailscale -and @('sim', 'nao') -notcontains $Tailscale) { Stop-Bootstrap '-Tailscale aceita sim ou nao' }
if (-not $destino) { $destino = $env:CP_DESTINO }
if (-not $destino) { $destino = Join-Path $HOME 'hangar' }

# $true = o remoto desta pasta e o hangar. Checar o REMOTO, nao so a existencia da
# pasta: "tem um .git aqui" nao quer dizer que e este projeto, e dar pull no repo errado e pior
# que parar.
function EhEsteRepo($pasta) {
    if (-not (Test-Path (Join-Path $pasta '.git'))) { return $false }
    $url = & git -C $pasta remote get-url origin 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $url) { return $false }
    $url = ([string]$url).Trim().TrimEnd('/')
    if ($url.EndsWith('.git')) { $url = $url.Substring(0, $url.Length - 4) }
    return ($url -match 'github\.com[:/]jeffer1312/hangar$')
}

function Clona {
    Titulo "Clonando em $destino"
    & git clone --branch $ramo $repoUrl $destino
    if ($LASTEXITCODE -ne 0) { Stop-Bootstrap 'git clone falhou' (Get-NetworkCode) }
    Ok 'clonado'
}

Titulo 'hangar - instalacao em uma linha'

Atualiza-Path
if (Tem 'git') {
    Ok 'git'
} else {
    # Aqui o git nao e opcional como no install.ps1 (la ele so alimenta o painel de branch):
    # sem git nao ha como clonar, e nao ha o que instalar depois.
    if (-not (Tem 'winget')) {
        Nota 'https://git-scm.com/download/win - e rode esta linha de novo'
        Stop-Bootstrap 'sem git e sem winget - instale o "App Installer" pela Microsoft Store, ou o Git' 'sem-winget'
    }
    Write-Host '  .. instalando Git (Git.Git)'
    winget install --id Git.Git --exact --silent `
        --accept-package-agreements --accept-source-agreements 2>&1 | Out-Null
    Atualiza-Path
    if (-not (Tem 'git')) { Stop-Bootstrap 'o Git nao instalou - instale na mao e rode de novo' (Get-NetworkCode) }
    Ok 'Git instalado'
}

if (-not (Test-Path $destino)) {
    Clona
} elseif (EhEsteRepo $destino) {
    Ok "$destino ja e este repositorio - atualizando em vez de clonar"
    & git -C $destino pull --ff-only origin $ramo
    if ($LASTEXITCODE -ne 0) {
        # Mudanca local e o caso comum, e tem frase e botao proprios no app.
        $mudancas = & git -C $destino status --porcelain --untracked-files=no 2>$null
        if ($mudancas) { Stop-Bootstrap "git pull falhou em $destino (ha mudanca local) - resolva na mao e rode de novo" 'checkout-sujo' }
        Stop-Bootstrap "git pull falhou em $destino - resolva na mao e rode de novo" (Get-NetworkCode)
    }
    Ok 'atualizado'
} elseif (Get-ChildItem -Force -LiteralPath $destino -ErrorAction SilentlyContinue) {
    Nota 'outro destino: $env:CP_DESTINO = "D:\hangar" e rode de novo'
    Stop-Bootstrap "$destino ja existe e NAO e o hangar - nao vou mexer no que e seu"
} else {
    Clona   # pasta existe mas esta vazia: o git clona pra dentro dela
}

Titulo 'Instalando'
$instalador = Join-Path $destino 'install.ps1'
if (-not (Test-Path $instalador)) { Stop-Bootstrap "install.ps1 nao encontrado em $destino" }
Set-Location $destino
# O que o app (ou quem chamou por -File) pediu vai inteiro ao install.ps1.
$repasse = @()
if ($App)       { $repasse += '-App' }
if ($Tailscale) { $repasse += @('-Tailscale', $Tailscale) }
if ($SemNativo) { $repasse += '-SemNativo' }
if ($Agentes)   { $repasse += @('-Agentes', $Agentes) }
if ($SoChecar)  { $repasse += '-SoChecar' }
if ($Sim)       { $repasse += '-Sim' }
if ($Avancado)  { $repasse += '-Avancado' }
if ($ConsertarRoda) { $repasse += '-ConsertarRoda' }
# Num processo proprio com -ExecutionPolicy Bypass: sob `irm | iex` a politica desta sessao
# pode ser Restricted, e ai um script EM ARQUIVO nao roda. O console e o mesmo, entao os
# Read-Host do install.ps1 continuam perguntando a voce normalmente.
# Pelo $PSHOME, nao pelo PATH: e o proprio host que esta rodando este texto. Sob pwsh 7 o
# $PSHOME nao tem powershell.exe, e ai vale o caminho fixo do 5.1, que todo Windows tem.
$ps = Join-Path $PSHOME 'powershell.exe'
if (-not (Test-Path $ps)) { $ps = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" }
& $ps -NoProfile -ExecutionPolicy Bypass -File $instalador @repasse
exit $LASTEXITCODE
