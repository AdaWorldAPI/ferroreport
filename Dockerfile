# ferroreport HTTP API: docker run -p 8080:8080 ghcr.io/cristianencarnacion/ferroreport
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p ferroreport-cli

FROM scratch
COPY --from=build /src/target/release/ferroreport /ferroreport
ENV FERROREPORT_HOST=0.0.0.0 FERROREPORT_PORT=8080
EXPOSE 8080
USER 65532:65532
ENTRYPOINT ["/ferroreport"]
CMD ["serve"]
