# --- Stage 1: Builder ---
FROM rust:1.75-slim-bookworm as builder

# Instalar dependencias de compilaciÃƒÂ³n
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/proxyia
COPY . .

# Compilar para release
RUN cargo build --release

# --- Stage 2: Runtime ---
FROM debian:bookworm-slim

# Instalar librerÃƒÂ­as de tiempo de ejecuciÃƒÂ³n
RUN apt-get update && apt-get install -y \
    libsqlite3-0 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Crear directorio para la aplicaciÃƒÂ³n y datos
WORKDIR /app

# Copiar el binario desde el builder
COPY --from=builder /usr/src/proxyia/target/release/ProxyIA /app/proxyia

# Copiar el esquema SQL y archivos necesarios
COPY sql/ /app/sql/

# Crear volumen para la base de datos y logs
# Esto permite que los datos no se borren al reiniciar el contenedor
VOLUME ["/app/data"]

# Variables de entorno por defecto
ENV DATABASE_URL="sqlite:/app/data/kairos.db"
ENV TOKEN_SAVINGS_LOG="true"

# ProxyIA usa STDIO para MCP
CMD ["/app/proxyia"]
