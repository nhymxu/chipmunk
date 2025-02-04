import FileParserText from './file.parser.text';
import FileParserDlt from './file.parser.dlt';
import FileParserPcapDlt from './file.parser.pcap.dlt';
import { AFileParser } from './interface';

export { FileParserText, FileParserDlt, FileParserPcapDlt };

export interface IFileParser {
    name: string;
    alias: string;
    desc: string;
    class: any;
    defaults?: boolean;
}
const FileParsers: IFileParser[] = [
    {
        name: 'Text',
        alias: 'text',
        desc: 'Parser for text files (utf8, ascii)',
        class: FileParserText,
        defaults: true,
    },
    {
        name: 'DLT',
        alias: 'dlt',
        desc: 'Parser for DLT files',
        class: FileParserDlt,
    },
    {
        name: 'DLT',
        alias: 'pcap_dlt',
        desc: 'Parser for DLT files, wrapped PCAP',
        class: FileParserPcapDlt,
    },
];

export function getDefaultFileParser(): AFileParser | undefined {
    let parser: AFileParser | undefined;
    FileParsers.forEach((desc) => {
        if (parser !== undefined) {
            return;
        }
        if (desc.defaults !== true) {
            return;
        }
        parser = new desc.class();
    });
    return parser;
}

export function getParserForFile(
    file: string,
    predefined?: AFileParser,
    defaults?: boolean,
): Promise<AFileParser | undefined> {
    return new Promise((resolve, reject) => {
        if (predefined !== undefined) {
            return resolve(predefined);
        }
        const parsers: AFileParser[] = FileParsers.map((desc) => {
            return new desc.class();
        });
        Promise.all(
            parsers.map((parser: AFileParser) => {
                return parser.isSupported(file);
            }),
        )
            .then((results: boolean[]) => {
                const index: number = results.indexOf(true);
                if (index === -1) {
                    resolve(getDefaultFileParser());
                }
                resolve(parsers[index]);
            })
            .catch((error: Error) => {
                reject(
                    new Error(
                        `Fail to file file parser for file "${file}" due error: ${error.message}`,
                    ),
                );
            });
    });
}

export { FileParsers, AFileParser };
