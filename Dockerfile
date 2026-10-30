FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p sryzamc
FROM debian:bookworm-slim
RUN useradd --system --create-home --uid 10001 sryza
WORKDIR /srv/sryza
COPY --from=build /src/target/release/sryzamc /usr/local/bin/sryzamc
USER 10001
EXPOSE 25565/tcp
ENTRYPOINT ["sryza", "--dir", "/srv/sryza"]
CMD ["run"]
