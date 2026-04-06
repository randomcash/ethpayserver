# ethpayserver-client - Leptos WASM frontend
#
# Build from ethpayserver directory:
#   docker build -f docker/client.Dockerfile -t ethpayserver-client .

FROM rust:1.90.0-bookworm AS builder

ARG HOME
ENV HOME=$HOME

WORKDIR /usr/local/app/

# Install trunk and wasm target
RUN cargo install trunk && \
    rustup target add wasm32-unknown-unknown

COPY . .

# Remove local patch (use git dependencies instead)
RUN sed -i '/^\[patch\./,/^$/d' Cargo.toml && \
    sed -i 's/"client"//' Cargo.toml

# Fetch payserver-commons for ui-kit and types dependencies
RUN \
    --mount=type=secret,id=GIT_AUTH_TOKEN \
    --mount=type=cache,target=$HOME/.cargo \
    if [ -f /run/secrets/GIT_AUTH_TOKEN ]; then \
        TOKEN=$(cat /run/secrets/GIT_AUTH_TOKEN) && \
        git config --global credential.helper store && \
        echo "https://x-access-token:${TOKEN}@gitlab.com" > ~/.git-credentials && \
        git config --global url."https://x-access-token:${TOKEN}@gitlab.com/".insteadOf "https://gitlab.com/" ; \
    fi && \
    git clone --depth 1 https://gitlab.com/random.cash/payserver-commons.git ../payserver-commons

# Build the WASM frontend
RUN \
    --mount=type=cache,target=./target \
    --mount=type=cache,target=$HOME/.cargo \
    cd client && trunk build --release

FROM nginx:alpine AS runtime
COPY --from=builder /usr/local/app/client/dist/ /usr/share/nginx/html/
COPY docker/client-nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE 80
