# aka's load balancer: angie (an nginx fork by the original nginx
# developers). Unlike plain nginx, angie ships the `stream` proxy in the
# open-source build, so aka can route raw TCP and TLS-by-SNI, not just HTTP.
FROM docker.angie.software/angie:1.12.2

# The packaged angie.conf ships the stream context commented out
# (`#stream {` / `#}` around the stream.d include); aka renders stream
# servers there, so enable the context at build time (asserted so pattern
# drift fails the build instead of silently disabling tcp routing).
# busybox sed: ERE, literal braces escaped, [[:space:]] (no \s)
RUN sed -i -E -e 's|^#stream \{$|stream {|' -e 's|^#\}$|}|' \
    -e 's|^#[[:space:]]*include /etc/angie/stream\.d/\*\.conf;|include /etc/angie/stream.d/*.conf;|' \
    /etc/angie/angie.conf \
    && grep -q '^stream {$' /etc/angie/angie.conf \
    && grep -q '^include /etc/angie/stream.d/\*\.conf;$' /etc/angie/angie.conf


# Route configs are rendered at runtime by `aka _proxyd` into bind-mounted
# directories; clear whatever the base image ships.
RUN rm -f /etc/angie/http.d/*.conf /etc/angie/stream.d/*.conf \
    && mkdir -p /etc/angie/http.d /etc/angie/stream.d
