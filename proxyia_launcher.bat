@echo off
REM ============================================================
REM ProxyIA Launcher v2.1 - Inicia el MCP + inyecta instrucciones
REM ============================================================
setlocal enabledelayedexpansion

set PROXYIA_BIN=C:\Proyectos\Proyectos_Rust\ProxyIAv2\ProxyIA.exe
set PROXYIA_DIR=C:\Proyectos\Proyectos_Rust\ProxyIAv2

:: Verificar que ProxyIA existe
if not exist "%PROXYIA_BIN%" (
    echo [ERROR] No se encuentra ProxyIA.exe en %PROXYIA_BIN%
    pause
    exit /b 1
)

:: 1. Matar instancias previas de ProxyIA (opcional)
taskkill /F /IM ProxyIA.exe 2>nul >nul

:: 2. Iniciar ProxyIA en background
echo [INFO] Iniciando ProxyIA MCP...
start /B "" /D "%PROXYIA_DIR%" "%PROXYIA_BIN%"

:: 3. Esperar a que arranque
timeout /t 2 /nobreak >nul

:: 4. Verificar que está corriendo
tasklist /FI "IMAGENAME eq ProxyIA.exe" 2>nul | find /I "ProxyIA.exe" >nul
if errorlevel 1 (
    echo [ERROR] ProxyIA no pudo iniciarse.
    pause
    exit /b 1
)
echo [OK] ProxyIA MCP corriendo en background.

:: 5. Determinar qué CLI está instalado
echo.
echo ============================================================
echo    🚀 ProxyIA MCP - Launcher v2.1
echo ============================================================
echo.
echo Selecciona el cliente a lanzar:
echo   [1] Qwen CLI (con instrucciones ProxyIA pre-cargadas)
echo   [2] Gemini CLI (con instrucciones ProxyIA pre-cargadas)
echo   [3] Claude Desktop (ya usa ProxyIA si está configurado)
echo   [4] Salir
echo.

set /p CHOICE="Opcion: "

set "PROMPT_INSTRUCTIONS=IMPORTANTE: ProxyIA MCP esta activo. ANTES de leer archivos usa search_context. Usa get_structural_map para vision general. Usa explore_neighbors para archivos relacionados. NO leas archivos completos a menos que sea estrictamente necesario."

if "%CHOICE%"=="1" (
    echo [INFO] Lanzando Qwen con instrucciones ProxyIA...
    qwen chat -p "%PROMPT_INSTRUCTIONS%"
) else if "%CHOICE%"=="2" (
    echo [INFO] Lanzando Gemini con instrucciones ProxyIA...
    gemini chat -p "%PROMPT_INSTRUCTIONS%"
) else if "%CHOICE%"=="3" (
    echo [INFO] Lanzando Claude Desktop...
    start "" "%APPDATA%\Claude\claude.exe"
) else (
    echo [INFO] Saliendo. ProxyIA sigue corriendo en background.
    echo Para detenerlo: taskkill /F /IM ProxyIA.exe
)

:: Al salir del CLI, ofrecer detener ProxyIA
echo.
echo ProxyIA MCP sigue corriendo. Para detenerlo manualmente:
echo   taskkill /F /IM ProxyIA.exe
echo O presiona cualquier tecla para detenerlo ahora...
pause >nul
taskkill /F /IM ProxyIA.exe 2>nul >nul
echo [OK] ProxyIA detenido.