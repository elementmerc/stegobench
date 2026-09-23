# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: Apache-2.0
# Copyright (C) 2026 Daniel Iwugo
"""Find data hidden after the end of a picture, and say when it has not looked.

This plugin flags bytes that sit after the point where a PNG or JPEG says the
picture ends. That is one of the oldest ways to hide a payload in an image and
it survives being mailed, uploaded and downloaded, so it turns up in real
material.

THE PART WORTH COPYING IS NOT THE STEGANOGRAPHY
------------------------------------------------
It is the second property this plugin writes.

`stegStructural` says what was found. `stegAssessed` says whether the question
was asked at all. A file that does not parse, or is not a picture, or is too
large to read, gets `not assessed` with a reason, never `none`.

The distinction matters because `none` and `not assessed` look identical in a
result list and mean opposite things to an investigator. "We looked and found
nothing" closes a line of enquiry. "We could not look" should open one. Any
plugin whose method has a stated scope has this problem, and it is cheap to
get right: one extra property and a refusal to guess.

WHAT COUNTS AS THE END OF A PICTURE
-------------------------------------
PNG ends at the IEND chunk's CRC. JPEG ends at the EOI marker, and finding
that one is the only fiddly part: see `trailing_data.py`, which explains why
searching for the two bytes FF D9 gets it wrong on an ordinary photograph.
"""
from hansken_extraction_plugin.api.extraction_plugin import ExtractionPlugin
from hansken_extraction_plugin.api.plugin_info import (
    Author,
    MaturityLevel,
    PluginId,
    PluginInfo,
    PluginResources,
)
from hansken_extraction_plugin.runtime.extraction_plugin_runner import run_with_hanskenpy
from logbook import Logger

import trailing_data

log = Logger(__name__)

#: Refuse to read a picture larger than this into memory. Evidence contains
#: files that claim to be pictures and are not, and an unbounded read is how
#: one exhibit takes down a worker.
MAX_PICTURE_BYTES = 64 * 1024 * 1024

#: Properties are camel case, as the SDK examples README requires.
NS = 'picture.misc'


class SteganographyPlugin(ExtractionPlugin):
    def plugin_info(self):
        return PluginInfo(
            id=PluginId(domain='stegcore.dev', category='picture', name='AppendedDataDetection'),
            version='1.0.0',
            description=(
                'Example Extraction Plugin: finds data appended after the logical end of a '
                'PNG or JPEG, and records explicitly when a picture could not be assessed '
                'rather than reporting it as clean.'
            ),
            author=Author('Daniel Iwugo', 'daniel@themalwarefiles.com', 'The Malware Files'),
            maturity=MaturityLevel.PROOF_OF_CONCEPT,
            webpage_url='https://github.com/The-Malware-Files/Stegcore',
            matcher='type=picture AND $data.type=raw',
            license='Apache License 2.0',
            resources=PluginResources(maximum_cpu=1, maximum_memory=256, maximum_workers=4),
        )

    def process(self, trace, data_context):
        name = str(trace.get('file.name') or trace.get('name') or '<unnamed>')
        size = data_context.data_size

        if size > MAX_PICTURE_BYTES:
            self._not_assessed(trace, name, f'{size} bytes exceeds the {MAX_PICTURE_BYTES} byte ceiling')
            return

        with trace.open() as reader:
            data = reader.read(size)

        try:
            found = trailing_data.find_trailing(data)
        except trailing_data.PrependedData as exc:
            # Bytes in FRONT of the picture. A different hiding place, and
            # reporting it as "not a picture" would lose the finding.
            log.info(f'{name}: {exc}')
            trace.update({
                f'{NS}.stegAssessed': 'yes',
                f'{NS}.stegStructural': 'prepended data',
                f'{NS}.stegPrependedBytes': str(exc.offset),
            })
            return
        except trailing_data.MalformedImage as exc:
            # A file that does not parse has not been cleared.
            self._not_assessed(trace, name, str(exc))
            return
        except ValueError:
            self._not_assessed(trace, name, 'not a PNG or JPEG')
            return

        if not found.present:
            trace.update({f'{NS}.stegAssessed': 'yes', f'{NS}.stegStructural': 'none'})
            return

        described = found.looks_like or 'unrecognised data'
        log.info(f'{name}: {found.length} bytes after the picture ends ({described})')
        trace.update({
            f'{NS}.stegAssessed': 'yes',
            f'{NS}.stegStructural': 'appended data',
            f'{NS}.stegAppendedBytes': str(found.length),
            f'{NS}.stegAppendedOffset': str(found.offset),
            f'{NS}.stegAppendedLooksLike': described,
        })

    @staticmethod
    def _not_assessed(trace, name, reason):
        """Record that nothing was measured, and why.

        Deliberately not `stegStructural = none`. See the module docstring.
        """
        log.info(f'{name}: not assessed ({reason})')
        trace.update({f'{NS}.stegAssessed': f'no: {reason}'})


if __name__ == '__main__':
    run_with_hanskenpy(SteganographyPlugin)
