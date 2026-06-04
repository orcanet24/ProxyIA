@echo off
SETLOCAL ENABLEDELAYEDEXPANSION

REM ============================================================
REM  build_and_deploy.bat — Compila ProxyIA y despliega el .exe
REM  Uso:  doble clic  o  build_and_deploy.bat
REM ============================================================

set PROJECT_DIR=C:\Proyectos\Proyectos_Rust\ProxyIAv2
set BINARY=%PROJECT_DIR%\target\release\ProxyIA.exe
set DEPLOY=%PROJECT_DIR%\ProxyIA.exe

echo ========================================
echo  🚀 Compilando ProxyIA (release)...
echo ========================================

cd /d "%PROJECT_DIR%"
if %ERRORLEVEL% neq 0 (
    echo [ERROR] No se pudo acceder a %PROJECT_DIR%
    pause
    exit /b 1
)

REM Compilar
call cargo build --release
if %ERRORLEVEL% neq 0 (
    echo [ERROR] La compilación falló. Revisa los errores arriba.
    pause
    exit /b 1
)

echo.
echo ========================================
echo  📦 Desplegando binario...
echo ========================================

REM Intentar copiar (falla si el .exe está en uso por otro proceso)
:retry_copy
copy /Y "%BINARY%" "%DEPLOY%" >nul 2>&1
if %ERRORLEVEL% equ 0 (
    echo ✅ Binario copiado a: %DEPLOY%
    goto :done
)

REM Si falló, preguntar si matar el proceso
echo ⚠️  No se pudo copiar. El archivo ProxyIA.exe está en uso.
echo    POSIBLES CAUSAS:
echo      - El servidor MCP está corriendo en otro cliente (Qwen, Gemini, etc.)
echo      - El .exe se ejecutó manualmente y no se cerró
echo.
set /p KILL="¿Cerrar el proceso ProxyIA.exe? (s/N): "
if /I "!KILL!"=="s" (
    echo.
    echo Cerrando proceso...
    taskkill /f /im ProxyIA.exe >nul 2>&1
    if !ERRORLEVEL! equ 0 (
        echo ✅ Proceso terminado.
        timeout /t 1 /nobreak >nul
        goto :retry_copy
    ) else (
        echo [ERROR] No se pudo terminar el proceso.
        echo         Ciérralo manualmente y vuelve a ejecutar este script.
    )
) else (
    echo.
    echo Puedes copiarlo manualmente con:
    echo   copy /Y "%BINARY%" "%DEPLOY%"
)

:done
echo.
echo ========================================
echo  ✅ Listo.
echo  Versión: %PROJECT_DIR%
echo  Binario: %DEPLOY%
echo ========================================
pause
exit /b 0
