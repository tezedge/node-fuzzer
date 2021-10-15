#!/bin/sh

mkdir coverage
mkdir log
cargo build --release
docker network create --subnet=172.18.0.0/16 fuzznet
ip -4 route add local 172.28.0.0/16 dev lo
docker build -t fuzz_target .
iptables -I DOCKER-USER -s 172.18.0.101 -p tcp --dport 9732 -j REJECT --reject-with tcp-reset
