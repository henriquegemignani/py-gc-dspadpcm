
import argparse
import pathlib
from . import encode_wav

parser = argparse.ArgumentParser()
parser.add_argument("input", type=pathlib.Path)
parser.add_argument("output", type=pathlib.Path)
args = parser.parse_args()


args.output.write_bytes(encode_wav(
    args.input.read_bytes()
))