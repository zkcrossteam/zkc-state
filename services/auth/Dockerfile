FROM golang:1.20-alpine as builder

WORKDIR /app

COPY server ./server
COPY go.mod go.sum ./
RUN go mod download

COPY ./ ./
RUN go build -o /auth

FROM alpine:3.18
COPY --from=builder /auth /auth
CMD ["/auth"]
