/* main.c — CLI wrapper for the DSP-ADPCM reference encoder by jackoalan, MIT licence
 * https://github.com/jackoalan/gc-dspadpcm-encode
 * Lightly trimmed: ALSA_PLAY and WRITE_WAV paths removed for portability.
 */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <math.h>

void DSPCorrelateCoefs(const short* source, int samples, short* coefs);
void DSPEncodeFrame(short* source, int samples, unsigned char* dest, const short coefs[8][2]);

#define MIN(a,b) (((a)<(b))?(a):(b))

#define PACKET_SAMPLES 14
#define PACKET_BYTES 8

/* Standard DSPADPCM header */
struct dspadpcm_header
{
    uint32_t num_samples;
    uint32_t num_nibbles;
    uint32_t sample_rate;
    uint16_t loop_flag;
    uint16_t format;
    uint32_t loop_start;
    uint32_t loop_end;
    uint32_t ca;
    int16_t coef[16];
    int16_t gain;
    int16_t ps;
    int16_t hist1;
    int16_t hist2;
    int16_t loop_ps;
    int16_t loop_hist1;
    int16_t loop_hist2;
    uint16_t pad[11];
};

static uint32_t bswap32(uint32_t v) {
    return ((v & 0xFF000000) >> 24) | ((v & 0x00FF0000) >> 8)
         | ((v & 0x0000FF00) << 8)  | ((v & 0x000000FF) << 24);
}
static uint16_t bswap16(uint16_t v) {
    return (uint16_t)(((v & 0xFF00) >> 8) | ((v & 0x00FF) << 8));
}

static int GetNibbleFromSample(int samples)
{
    int packets = samples / PACKET_SAMPLES;
    int extraSamples = samples % PACKET_SAMPLES;
    int extraNibbles = extraSamples == 0 ? 0 : extraSamples + 2;
    return 16 * packets + extraNibbles;
}

static int GetNibbleAddress(int sample)
{
    int packets = sample / PACKET_SAMPLES;
    int extraSamples = sample % PACKET_SAMPLES;
    return 16 * packets + extraSamples + 2;
}

static int GetBytesForAdpcmSamples(int samples)
{
    int extraBytes = 0;
    int packets = samples / PACKET_SAMPLES;
    int extraSamples = samples % PACKET_SAMPLES;
    if (extraSamples != 0)
        extraBytes = (extraSamples / 2) + (extraSamples % 2) + 1;
    return PACKET_BYTES * packets + extraBytes;
}

int main(int argc, char** argv)
{
    int i, p, s;

    if (argc < 3)
    {
        fprintf(stderr, "Usage: %s <wavin> <dspout>\n", *argv);
        return 1;
    }

    FILE* fin = fopen(argv[1], "rb");
    if (!fin) { fprintf(stderr, "'%s': %s\n", argv[1], strerror(errno)); return 1; }

    char riffcheck[4];
    fread(riffcheck, 1, 4, fin);
    if (memcmp(riffcheck, "RIFF", 4)) { fprintf(stderr, "Not a RIFF file\n"); fclose(fin); return 1; }
    fseek(fin, 4, SEEK_CUR);
    fread(riffcheck, 1, 4, fin);
    if (memcmp(riffcheck, "WAVE", 4)) { fprintf(stderr, "Not a WAVE file\n"); fclose(fin); return 1; }

    uint32_t samplerate = 0;
    uint32_t samplecount = 0;
    while (fread(riffcheck, 1, 4, fin) == 4)
    {
        uint32_t chunkSz;
        fread(&chunkSz, 1, 4, fin);
        /* assume little-endian host (x86/x64) */
        if (!memcmp(riffcheck, "fmt ", 4))
        {
            uint16_t fmt; fread(&fmt, 1, 2, fin);
            if (fmt != 1) { fprintf(stderr, "Non-PCM WAV not supported\n"); fclose(fin); return 1; }
            uint16_t nchan; fread(&nchan, 1, 2, fin);
            if (nchan != 1) { fprintf(stderr, "Only mono WAV supported\n"); fclose(fin); return 1; }
            fread(&samplerate, 1, 4, fin);
            fseek(fin, 4, SEEK_CUR);
            uint16_t bps; fread(&bps, 1, 2, fin);
            uint16_t bpsamp; fread(&bpsamp, 1, 2, fin);
            if (bpsamp != 16) { fprintf(stderr, "Only 16-bit WAV supported\n"); fclose(fin); return 1; }
        }
        else if (!memcmp(riffcheck, "data", 4))
        {
            samplecount = chunkSz / 2;
            break;
        }
        else
            fseek(fin, chunkSz, SEEK_CUR);
    }

    if (!samplerate || !samplecount) {
        fprintf(stderr, "Invalid WAV\n"); fclose(fin); return 1;
    }

    int packetCount = samplecount / PACKET_SAMPLES + (samplecount % PACKET_SAMPLES != 0);
    int16_t* sampsBuf = (int16_t*)calloc(samplecount, sizeof(int16_t));
    fread(sampsBuf, samplecount, 2, fin);
    fclose(fin);

    int16_t coefs[16];
    DSPCorrelateCoefs(sampsBuf, samplecount, coefs);

    FILE* fout = fopen(argv[2], "wb");
    if (!fout) { fprintf(stderr, "'%s': %s\n", argv[2], strerror(errno)); free(sampsBuf); return 1; }

    struct dspadpcm_header header = {0};
    /* Write big-endian header fields */
    header.num_samples  = bswap32(samplecount);
    header.num_nibbles  = bswap32(GetNibbleFromSample(samplecount));
    header.sample_rate  = bswap32(samplerate);
    header.loop_start   = bswap32(GetNibbleAddress(0));
    header.loop_end     = bswap32(GetNibbleAddress(samplecount - 1));
    header.ca           = bswap32(GetNibbleAddress(0));
    for (i=0 ; i<16 ; ++i)
        header.coef[i] = bswap16(coefs[i]);

    int16_t convSamps[16] = {0};
    unsigned char block[8];

    for (p=0 ; p<packetCount ; ++p)
    {
        memset(convSamps + 2, 0, PACKET_SAMPLES * sizeof(int16_t));
        int numSamples = MIN(samplecount - p * PACKET_SAMPLES, PACKET_SAMPLES);
        for (s=0 ; s<numSamples ; ++s)
            convSamps[s+2] = sampsBuf[p*PACKET_SAMPLES+s];

        DSPEncodeFrame(convSamps, PACKET_SAMPLES, block, (const short(*)[2])coefs);

        convSamps[0] = convSamps[14];
        convSamps[1] = convSamps[15];

        if (p == 0)
        {
            header.ps = bswap16(block[0]);
            fwrite(&header, 1, sizeof(header), fout);
        }

        fwrite(block, 1, GetBytesForAdpcmSamples(numSamples), fout);
    }

    fclose(fout);
    free(sampsBuf);
    return 0;
}
