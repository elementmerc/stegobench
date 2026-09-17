# StegExpose (b3dk7) — LSB steganalysis ensemble (Sample Pairs, RS, Chi-square,
# Primary Sets) for lossless images. Detector. Distributed as a jar.
FROM eclipse-temurin:17-jre-jammy

ARG STEGEXPOSE_REF=master
RUN apt-get update \
    && apt-get install -y --no-install-recommends git \
    && git clone https://github.com/b3dk7/StegExpose /opt/stegexpose \
    && cd /opt/stegexpose \
    && git checkout "${STEGEXPOSE_REF}" \
    && apt-get purge -y git && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /data
# StegExpose scans a DIRECTORY of images and prints a CSV verdict per file:
#   java -jar /opt/stegexpose/StegExpose.jar /data
ENTRYPOINT ["java", "-jar", "/opt/stegexpose/StegExpose.jar"]
CMD []
