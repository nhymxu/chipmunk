import * as path from 'path';
import * as Tools from '../../tools/index';

import indexer from 'indexer-neon';
import ServiceOutputExport from '../../services/output/service.output.export';
import ServiceNotifications, { ENotificationType } from '../../services/service.notifications';
import ServiceStreams from '../../services/service.streams';
import Logger from '../../tools/env.logger';
import ServiceElectron from '../../services/service.electron';
import ServiceDLTFiles from '../../services/parsers/service.dlt.files';

import { AFileParser, IMapItem } from './interface';
import { DLT, Progress, CancelablePromise } from 'indexer-neon';
import { CommonInterfaces } from '../../interfaces/interface.common';
import { CExportAllActionId } from '../../consts/output.actions';
import { dialog, SaveDialogReturnValue } from 'electron';
import { IPCMessages } from '../../services/service.electron';

export const CMetaData = 'dlt';

const ExtNames = ['pcapng'];

export default class FileParser extends AFileParser {
    private _guid: string | undefined;
    private _logger: Logger = new Logger('DLT PCAP Indexing');
    private _task:
        | CancelablePromise<void, void, DLT.TIndexDltAsyncEvents, DLT.TIndexDltAsyncEventObject>
        | undefined;
    private _saves: Map<
        string,
        CancelablePromise<void, void, DLT.TPcap2DltEvents, DLT.TPcap2DltsEventObject>
    > = new Map();

    constructor() {
        super();
    }

    public static getAlias(): string {
        return 'pcap_dlt';
    }

    public destroy(): Promise<void> {
        return this.abort();
    }

    public getName(): string {
        return 'DLT format (PCAP wrapped)';
    }

    public getAlias(): string {
        return FileParser.getAlias();
    }

    public getMeta(): string {
        return CMetaData;
    }

    public getExtnameFilters(): Array<{ name: string; extensions: string[] }> {
        return [{ name: 'DLT Files wrapped into PCAP', extensions: ExtNames }];
    }

    public getExtensions(): string[] {
        return ExtNames.slice();
    }

    public isSupported(file: string): Promise<boolean> {
        return new Promise((resolve, reject) => {
            const extname: string = path.extname(file).toLowerCase().replace('.', '');
            resolve(ExtNames.indexOf(extname) !== -1);
        });
    }

    public parseAndIndex(
        srcFile: string,
        destFile: string,
        sourceId: string,
        options: CommonInterfaces.DLT.IDLTOptions,
        onMapUpdated?: (map: IMapItem[]) => void,
        onProgress?: (ticks: Progress.ITicks) => void,
    ): Tools.CancelablePromise<IMapItem[], void> {
        return new Tools.CancelablePromise<IMapItem[], void>((resolve, reject, cancel) => {
            if (this._guid !== undefined) {
                return reject(new Error(`Parsing is already started.`));
            }
            this._guid = ServiceStreams.getActiveStreamId();
            const collectedChunks: IMapItem[] = [];
            const hrstart = process.hrtime();
            const stats: CommonInterfaces.DLT.StatisticInfo | undefined =
                ServiceDLTFiles.getStats(srcFile);
            const appIdCount: number = stats === undefined ? -1 : stats.app_ids.length;
            const contextIdCount: number = stats === undefined ? -1 : stats.context_ids.length;
            let appIds: string[] | undefined;
            let contextIds: string[] | undefined;
            let ecuIds: string[] | undefined;
            if (options === undefined) {
                (options as any) = {
                    logLevel: 6,
                };
            }
            if (options.filters !== undefined && options.filters.app_ids instanceof Array) {
                appIds = options.filters.app_ids;
            }
            if (options.filters !== undefined && options.filters.context_ids instanceof Array) {
                contextIds = options.filters.context_ids;
            }
            if (options.filters !== undefined && options.filters.ecu_ids instanceof Array) {
                ecuIds = options.filters.ecu_ids;
            }
            if (typeof options.fibex !== 'object' || options.fibex === null) {
                options.fibex = {
                    fibex_file_paths: [],
                };
            }
            if (!(options.fibex.fibex_file_paths instanceof Array)) {
                options.fibex.fibex_file_paths = [];
            }
            const filterConfig: DLT.DltFilterConf = {
                min_log_level: options.logLevel,
                app_ids: appIds,
                context_ids: contextIds,
                ecu_ids: ecuIds,
                app_id_count: appIdCount,
                context_id_count: contextIdCount,
            };
            const dltParams: DLT.IIndexDltParams = {
                dltFile: srcFile,
                fibex: options.fibex,
                filterConfig,
                tag: sourceId.toString(),
                out: destFile,
                chunk_size: 500,
                append: false,
                stdout: false,
                statusUpdates: true,
                formatOptions: {
                    tz: options.tz,
                },
            };
            this._logger.debug('calling indexPcapDlt with params: ' + JSON.stringify(dltParams));
            this._task = indexer
                .indexPcapDlt(dltParams)
                .then(() => {
                    // Register exports callback
                    ServiceOutputExport.setAction(this._guid as string, {
                        id: CExportAllActionId,
                        caption: 'Save file as DLT',
                        handler: this._saveAsDLT.bind(this, srcFile),
                        isEnabled: () => true,
                        source: IPCMessages.EOutputExportFeaturesSource.output,
                    });
                    resolve(collectedChunks);
                })
                .catch((error: Error) => {
                    ServiceNotifications.notify({
                        message:
                            error.message.length > 1500
                                ? `${error.message.substr(0, 1500)}...`
                                : error.message,
                        caption: `Error with: ${path.basename(srcFile)}`,
                        session: this._guid,
                        file: sourceId.toString(),
                        type: ENotificationType.error,
                    });
                    reject(error);
                })
                .canceled(() => {
                    cancel();
                })
                .finally(() => {
                    this._task = undefined;
                    const hrend = process.hrtime(hrstart);
                    const ms = Math.round(hrend[0] * 1000 + hrend[1] / 1000000);
                    this._logger.debug('parseAndIndex task finished');
                    this._logger.debug('Execution time for indexing : ' + ms + 'ms');
                })
                .on('chunk', (event: Progress.IChunk) => {
                    if (onMapUpdated !== undefined) {
                        const mapItem: IMapItem = {
                            rows: { from: event.rowsStart, to: event.rowsEnd },
                            bytes: { from: event.bytesStart, to: event.bytesEnd },
                        };
                        onMapUpdated([mapItem]);
                        collectedChunks.push(mapItem);
                    }
                })
                .on('progress', (event: Progress.ITicks) => {
                    if (onProgress !== undefined) {
                        onProgress(event);
                    }
                })
                .on('notification', (event: Progress.INeonNotification) => {
                    ServiceNotifications.notifyFromNeon(
                        event,
                        'DLT-PCAP-Indexing',
                        this._guid,
                        srcFile,
                    );
                });
        });
    }

    public abort(): Promise<void> {
        return new Promise((resolve) => {
            this._saves.forEach((task) => task.abort());
            if (this._task === undefined) {
                return resolve();
            }
            this._task
                .canceled(() => {
                    resolve();
                })
                .abort();
        });
    }

    private _saveAsDLT(target: string): Promise<void> {
        return new Promise((resolve, reject) => {
            this._getFileName()
                .then((filename: string | undefined) => {
                    if (filename === undefined) {
                        return;
                    }
                    this._logger.info(`Saving`);
                    const guid: string = Tools.guid();
                    const task: CancelablePromise<
                        void,
                        void,
                        DLT.TPcap2DltEvents,
                        DLT.TPcap2DltsEventObject
                    > = indexer
                        .pcapToDlt(target, filename)
                        .then(() => {
                            this._logger.info(`Saved`);
                            resolve();
                        })
                        .canceled(() => {
                            this._logger.info(`Saving was canceled`);
                            resolve();
                        })
                        .catch((error: Error) => {
                            this._logger.warn(`Exception during saving: ${error.message}`);
                            reject(error);
                        })
                        .finally(() => {
                            this._saves.delete(guid);
                        })
                        .on('progress', (event: Progress.ITicks) => {
                            // TODO: Do we need this event at all?
                        });
                    this._saves.set(guid, task);
                })
                .catch((error: Error) => {
                    this._logger.warn(`Fail to select a file due error: ${error.message}`);
                });
        });
    }

    private _getFileName(): Promise<string | undefined> {
        return new Promise((resolve, reject) => {
            const win = ServiceElectron.getBrowserWindow();
            if (win === undefined) {
                return;
            }
            dialog
                .showSaveDialog(win, {
                    title: 'Converting PCAPNG to DLT',
                    filters: [
                        {
                            name: 'DLT Files',
                            extensions: ['dlt'],
                        },
                    ],
                })
                .then((returnValue: SaveDialogReturnValue) => {
                    resolve(returnValue.filePath);
                })
                .catch((error: Error) => {
                    this._logger.error(`Fail get filename for saving due error: ${error.message}`);
                    reject(error);
                });
        });
    }
}
