FROM gcr.io/oss-fuzz-base/base-builder-rust
RUN git clone --depth 1 https://github.com/cristianEncarnacion/ferroreport ferroreport
WORKDIR ferroreport
COPY build.sh $SRC/
