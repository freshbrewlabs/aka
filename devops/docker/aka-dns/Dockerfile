# aka's DNS server: dnsmasq with wildcard address records for the
# configured domains, pointed at the proxy container. The container keeps
# its own resolv.conf, so host DNS is untouched outside the resolver
# entries aka writes.
FROM alpine:3.22

RUN apk add --no-cache dnsmasq

# `aka up` passes the full command line via DNSMASQ_ARGS (one env var,
# word-split by the shell form below).
EXPOSE 53/tcp 53/udp

ENTRYPOINT ["sh", "-c", "exec dnsmasq --no-daemon ${DNSMASQ_ARGS:-}"]
