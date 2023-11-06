FROM docker.io/golang:1.20-alpine as builder

WORKDIR /app
ARG GOPROXY

COPY server ./server
COPY go.mod go.sum ./
RUN GOPROXY=${GOPROXY} go mod download

COPY ./ ./
RUN go build -o /auth

FROM docker.io/alpine:3.18
COPY --from=builder /auth /auth
CMD ["/auth"]
