# aka's public landing page: the static site in src/public, served by nginx.
# Context is the repo root (see aka-public-build.yml), same as aka-admin.
# No RUN steps: multi-arch is free.
FROM nginx:1.29-alpine

# Replace the stock :80 vhost with an unprivileged :8080 one, and the stock
# main config (root-owned /var paths) with a /tmp-based one: the container
# runs as uid 101 in k8s and can neither bind low ports nor write /var/cache.
COPY devops/docker/aka-public/nginx.conf /etc/nginx/nginx.conf
COPY devops/docker/aka-public/default.conf /etc/nginx/conf.d/default.conf
COPY src/public/ /usr/share/nginx/html/
