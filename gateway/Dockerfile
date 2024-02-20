FROM docker.io/golang:1.20-alpine as builder

WORKDIR /app
ARG GOPROXY=

COPY go.mod go.sum ./
COPY ./gen ./gen
RUN GOPROXY=${GOPROXY} go mod download

COPY ./ ./
RUN go build -o /gateway

FROM docker.io/alpine:3.18
COPY --from=builder /gateway /gateway
CMD ["/gateway"]
