FROM ubuntu:24.04

# Install Node.js 20
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates gnupg \
    && curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install Wick binary + libcronet.so from GitHub release
ARG WICK_VERSION=0.7.0
ARG WICK_SHA256=110d074072ff5fb334ca3d0123def3f9463d5298f9c6a48fa727a03d21f08ea9

RUN cd /tmp \
    && curl -fsSL "https://github.com/wickproject/wick/releases/download/v${WICK_VERSION}/wick-linux-amd64.tar.gz" -o wick.tar.gz \
    && echo "${WICK_SHA256}  wick.tar.gz" | sha256sum -c - \
    && tar xzf wick.tar.gz \
    && mv wick /usr/local/bin/wick \
    && mv libcronet.so /usr/local/lib/libcronet.so \
    && chmod +x /usr/local/bin/wick \
    && ldconfig \
    && rm wick.tar.gz

# Verify wick runs
RUN wick version

WORKDIR /app
COPY package.json .
RUN npm install --production
COPY . .

CMD ["node", "src/main.js"]
