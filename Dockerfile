FROM debian:buster
RUN apt update
EXPOSE 8000
RUN apt install -y git curl openssl libssl-dev pkg-config 
RUN apt install -y libsodium-dev clang libclang-dev llvm llvm-dev libev-dev
RUN apt install -y make lcov python3 python3-pip
RUN update-alternatives --install /usr/bin/python python /usr/bin/python3 1
RUN update-alternatives --install /usr/bin/pip pip /usr/bin/pip3 1
RUN pip install psutil matplotlib mpld3
RUN git clone https://github.com/tezedge/tezedge --branch develop
VOLUME /tezedge
VOLUME /coverage
COPY ./scripts /scripts
ENV RUSTUP_HOME=/rust
ENV CARGO_HOME=/cargo 
ENV PATH=/cargo/bin:/rust/bin:$PATH
RUN (curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain nightly-2021-08-04 --no-modify-path) && rustup default nightly-2021-08-04
RUN cargo install cargo-binutils
RUN rustup component add llvm-tools-preview
RUN apt-get install -y supervisor
ADD supervisord.conf /etc/supervisor/conf.d/supervisord.conf 
CMD /usr/bin/supervisord -c /etc/supervisor/conf.d/supervisord.conf
