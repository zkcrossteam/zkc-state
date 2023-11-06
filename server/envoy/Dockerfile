FROM docker.io/envoyproxy/envoy:v1.23-latest
COPY . /etc/envoy/
# COPY envoy.yaml /etc/envoy/envoy.yaml
RUN chmod go+r /etc/envoy/envoy.yaml
CMD ["--log-level", "debug", "--config-path", "/etc/envoy/envoy.yaml"]
