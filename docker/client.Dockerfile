# ethpayserver-client - Leptos WASM frontend
#
# Expects pre-built WASM bundle from CI artifacts:
#   client/dist/

FROM nginx:alpine
COPY client/dist/ /usr/share/nginx/html/
COPY docker/client-nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE 80
