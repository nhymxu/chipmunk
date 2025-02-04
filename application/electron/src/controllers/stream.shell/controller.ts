import { IPCMessages as IPC, Subscription } from '../../services/service.electron';
import { getElectronAppShellEnvVars, getDefShell, getShells, TEnvVars } from 'chipmunk.shell.env';
import { dialog, OpenDialogReturnValue } from 'electron';

import ServiceStorage from '../../services/service.storage';
import Process from './controller.process';
import ServiceElectron from '../../services/service.electron';
import ServiceFileRecent from '../../services/files/service.file.recent';

import Logger from '../../tools/env.logger';

import * as Tools from '../../tools/index';
import * as path from 'path';
import * as os from 'os';
import * as FS from '../../tools/fs';

export default class ControllerStreamShell {
    private _logger: Logger;
    private _guid: string;
    private _running: Map<string, Process> = new Map();
    private _terminated: Map<string, Process> = new Map();
    private _subscriptions: { [key: string]: Subscription } = {};
    private _presetTitle: string = '';
    private _env: {
        env: { [key: string]: string };
        def: { [key: string]: string };
        shell: string;
        shells: string[];
        pwd: string;
    } = {
        env: {},
        def: {},
        shell: '',
        shells: [],
        pwd: path.normalize(`${os.homedir()}`),
    };

    constructor(guid: string) {
        this._guid = guid;
        this._logger = new Logger(`ControllerStreamShell: ${guid}`);
        let error: Error | undefined;
        // This controller should not block loading of app. Even initialization is failed, it
        // should not throw an exception.
        Promise.all([
            getElectronAppShellEnvVars(process.execPath)
                .then((envs: TEnvVars) => {
                    this._env.env = Object.assign({}, envs);
                    this._env.def = Object.assign({}, envs);
                })
                .catch((e) => (error = e)),
            getShells()
                .then((_: string[]) => {
                    this._env.shells = _;
                })
                .catch((e) => (error = e)),
            getDefShell()
                .then((_: string) => {
                    this._env.shell = _;
                })
                .catch((e) => (error = e)),
        ])
            .then(() => {
                if (error !== undefined) {
                    this._logger.error(
                        `Fail to correctly load envvars due error: ${error.message}`,
                    );
                }
                this._logger.verbose(
                    `Next envvars are detected:\n\t${Object.keys(this._env.def)
                        .map((k) => k + ' = ' + this._env.def[k])
                        .filter((v) => v.trim() !== '=')
                        .join('\n\t')}`,
                );
                this._logger.verbose(
                    `Next shells are detected:\n\t${this._env.shells.join('\n\t')}`,
                );
                this._logger.verbose(`Current shell is:\n\t${this._env.shell}`);
                this._logger.verbose(`Current pwd is:\n\t${this._env.pwd}`);
                ServiceElectron.IPC.subscribe(
                    IPC.ShellEnvRequest,
                    this._ipc_ShellEnvRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellEnvRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellEnvRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellProcessListRequest,
                    this._ipc_ShellProcessListRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellProcessListRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellProcessListRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellSetEnvRequest,
                    this._ipc_ShellSetEnvRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellSetEnvRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellSetEnvRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellProcessRunRequest,
                    this._ipc_ShellProcessRunRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellProcessRunRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellProcessRunRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellProcessHistoryGetRequest,
                    this._ipc_ShellProcessHistoryGetRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellProcessHistoryGetRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellProcessHistoryGetRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellProcessBundleSetRequest,
                    this._ipc_ShellProcessBundleSetRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellProcessBundleSetRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellProcessBundleSetRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellProcessBundleRemoveRequest,
                    this._ipc_ShellProcessBundleRemoveRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellProcessBundleRemoveRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellProcessBundleRemoveRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellPresetSetRequest,
                    this._ipc_ShellPresetSetRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellPresetSetRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellPresetSetRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellPresetRemoveRequest,
                    this._ipc_ShellPresetRemoveRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellPresetRemoveRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellPresetRemoveRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellPresetGetRequest,
                    this._ipc_ShellPresetGetRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellPresetGetRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellPresetGetRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellLoadRequest,
                    this._ipc_ShellLoadRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellLoadRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellLoadRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellSaveRequest,
                    this._ipc_ShellSaveRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellSaveRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellSaveRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellPwdRequest,
                    this._ipc_ShellPwdRequest.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellPwdRequest = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellPwdRequest due error: ${err.message}`,
                        ),
                    );
                ServiceElectron.IPC.subscribe(
                    IPC.ShellRecentCommandsRemove,
                    this._ipc_ShellRecentCommandsRemove.bind(this) as any,
                )
                    .then((subscription: Subscription) => {
                        this._subscriptions.ShellRecentCommandsRemove = subscription;
                    })
                    .catch((err: Error) =>
                        this._logger.error(
                            `Fail to subscribe to ShellSaveRequest due error: ${err.message}`,
                        ),
                    );
            })
            .catch((err: Error) => {
                this._logger.error(`Unexpecting error on load envvars: ${err.message}`);
            });
    }

    public destroy(): Promise<void> {
        this._running.forEach((proc: Process) => {
            proc.removeAllListeners();
            proc.destroy();
        });
        this._running.clear();
        this._terminated.clear();
        return Promise.resolve();
    }

    private _ipc_ShellProcessRunRequest(
        request: IPC.ShellProcessRunRequest,
        response: (response: IPC.ShellProcessRunResponse) => Promise<void>,
    ) {
        const sendListOfRunning = () => {
            ServiceElectron.IPC.send(
                new IPC.ShellProcessListEvent({
                    session: this._guid,
                    processes: Array.from(this._running.values()).map((p) => p.getInfo()),
                }),
            );
        };
        const sendListOfTerminated = () => {
            ServiceElectron.IPC.send(
                new IPC.ShellProcessStoppedEvent({
                    session: this._guid,
                    processes: Array.from(this._terminated.values()).map((p) => p.getInfo()),
                }),
            );
        };
        const stored: string[] = ServiceStorage.get().get().recentCommands;
        if (stored.indexOf(request.command) === -1) {
            stored.unshift(request.command);
            ServiceStorage.get()
                .set({
                    recentCommands: stored,
                })
                .catch((err: Error) => {
                    this._logger.error(
                        `Failed to save command as recent due error: ${err.message}`,
                    );
                });
        }
        if (request.session !== this._guid) {
            return;
        }
        const guid: string = Tools.guid();
        const proc: Process = new Process(this._guid, {
            guid: guid,
            cmd: request.command,
            settings: {
                env: this._env.env,
                pwd: request.pwd,
                shell: request.shell,
            },
        });
        proc.on(Process.Events.destroy, () => {
            proc.removeAllListeners();
            proc.getInfo().stat.terminated = Date.now() - proc.getInfo().stat.created;
            this._terminated.set(guid, proc);
            this._running.delete(guid);
            sendListOfTerminated();
            sendListOfRunning();
        });
        this._running.set(guid, proc);
        proc.execute();
        response(new IPC.ShellProcessRunResponse({})).finally(() => {
            sendListOfRunning();
        });
    }

    private _ipc_ShellSetEnvRequest(
        request: IPC.ShellSetEnvRequest,
        response: (response: IPC.ShellSetEnvResponse) => Promise<void>,
    ) {
        if (request.session !== this._guid) {
            return;
        }
        if (request.shell !== undefined && this._env.shells.indexOf(request.shell) === -1) {
            return response(
                new IPC.ShellSetEnvResponse({
                    error: `Fail to find shell "${
                        request.shell
                    }". Available shells: ${this._env.shells.join('; ')}`,
                }),
            );
        }
        new Promise((resolve, reject) => {
            if (request.pwd === undefined) {
                return resolve(undefined);
            } else {
                FS.exist(request.pwd)
                    .then((exist: boolean) => {
                        if (exist) {
                            resolve(undefined);
                        } else {
                            reject(new Error(`Path "${request.pwd}" doesn't exist`));
                        }
                    })
                    .catch(reject);
            }
        })
            .then(() => {
                this._env.pwd = request.pwd !== undefined ? request.pwd : this._env.pwd;
                this._env.shell = request.shell !== undefined ? request.shell : this._env.shell;
                if (request.env !== undefined) {
                    this._env.env = Object.assign({}, this._env.def);
                    request.env.forEach((variable) => {
                        if (this._env.def[variable.variable] === undefined) {
                            this._env.env[variable.variable] = variable.value;
                        }
                    });
                }
                response(new IPC.ShellSetEnvResponse({}));
            })
            .catch((err: Error) => {
                response(
                    new IPC.ShellSetEnvResponse({
                        error: err.message,
                    }),
                );
            });
    }

    private _ipc_ShellProcessListRequest(
        request: IPC.ShellProcessListRequest,
        response: (response: IPC.ShellProcessListResponse) => Promise<void>,
    ) {
        if (request.session !== this._guid) {
            return;
        }
        response(
            new IPC.ShellProcessListResponse({
                session: this._guid,
                processes: Array.from(this._running.values()).map((proc) => proc.getInfo()),
            }),
        );
    }

    private _ipc_ShellEnvRequest(
        request: IPC.ShellEnvRequest,
        response: (response: IPC.ShellEnvResponse) => Promise<void>,
    ) {
        if (request.session !== this._guid) {
            return;
        }
        response(
            new IPC.ShellEnvResponse({
                env: this._env.env,
                shell: this._env.shell,
                shells: this._env.shells,
                pwd: this._env.pwd,
            }),
        );
    }

    private _ipc_ShellProcessHistoryGetRequest(
        request: IPC.ShellProcessHistoryGetRequest,
        response: (response: IPC.ShellProcessHistoryGetResponse) => Promise<void>,
    ) {
        if (request.session !== this._guid) {
            return;
        }
        response(
            new IPC.ShellProcessHistoryGetResponse({
                session: this._guid,
                processes: Array.from(this._terminated.values()).map((proc) => proc.getInfo()),
                bundles: ServiceStorage.get().get().recentBundles,
            }),
        );
    }

    private _ipc_ShellProcessBundleSetRequest(
        request: IPC.ShellProcessBundleSetRequest,
        response: (response: IPC.ShellProcessBundleSetResponse) => Promise<void>,
    ) {
        if (request.session !== this._guid) {
            return;
        }
        let error: undefined | Error;
        const bundles: IPC.IBundle[] = ServiceStorage.get().get().recentBundles;
        bundles.push(request.bundle);
        ServiceStorage.get()
            .set({
                recentBundles: bundles,
            })
            .catch((err: Error) => {
                this._logger.error(err.message);
                error = err;
            })
            .finally(() => {
                response(
                    new IPC.ShellProcessBundleSetResponse({
                        error: error === undefined ? error : error.message,
                    }),
                );
            });
    }

    private _ipc_ShellProcessBundleRemoveRequest(
        request: IPC.ShellProcessBundleRemoveRequest,
        response: (response: IPC.ShellProcessBundleRemoveResponse) => Promise<void>,
    ) {
        if (request.session !== this._guid) {
            return;
        }
        let error: undefined | Error;
        let bundles: IPC.IBundle[] = ServiceStorage.get().get().recentBundles;
        request.bundles.forEach((rBundle: IPC.IBundle) => {
            bundles = bundles.filter((bundle: IPC.IBundle) => {
                return rBundle.title !== bundle.title;
            });
        });
        ServiceStorage.get()
            .set({
                recentBundles: bundles,
            })
            .catch((err: Error) => {
                this._logger.error(err.message);
                error = err;
            })
            .finally(() => {
                response(
                    new IPC.ShellProcessBundleRemoveResponse({
                        error: error === undefined ? error : error.message,
                    }),
                );
            });
    }

    private _ipc_ShellPresetSetRequest(
        request: IPC.ShellPresetSetRequest,
        response: (response: IPC.ShellPresetSetResponse) => Promise<void>,
    ) {
        if (request.session !== this._guid) {
            return;
        }
        if (
            request.preset.information.env !== undefined &&
            request.preset.information.pwd !== undefined &&
            request.preset.information.shell !== undefined
        ) {
            ServiceFileRecent.savePreset(request.preset);
        } else {
            ServiceFileRecent.modifyPreset(request.preset);
        }
        response(new IPC.ShellPresetSetResponse());
    }

    private _ipc_ShellPresetRemoveRequest(
        request: IPC.ShellPresetRemoveRequest,
        response: (response: IPC.ShellPresetRemoveResponse) => Promise<void>,
    ) {
        if (request.session !== this._guid) {
            return;
        }
        ServiceFileRecent.removePreset(request.title);
        response(new IPC.ShellPresetRemoveResponse());
    }

    private _ipc_ShellPresetGetRequest(
        request: IPC.ShellPresetGetRequest,
        response: (response: IPC.ShellPresetGetResponse) => Promise<IPC.IPreset[]>,
    ) {
        if (request.session !== this._guid) {
            return;
        }
        response(
            new IPC.ShellPresetGetResponse({
                session: this._guid,
                presets: ServiceFileRecent.loadPreset(),
            }),
        );
    }

    private _ipc_ShellLoadRequest(
        request: IPC.ShellLoadRequest,
        response: (response: IPC.ShellLoadResponse) => Promise<string>,
    ) {
        if (request.session !== this._guid) {
            return;
        }
        response(
            new IPC.ShellLoadResponse({ session: this._guid, presetTitle: this._presetTitle }),
        );
    }

    private _ipc_ShellSaveRequest(
        request: IPC.ShellSaveRequest,
        response: (response: IPC.ShellSaveResponse) => Promise<void>,
    ) {
        if (request.session !== this._guid) {
            return;
        }
        this._presetTitle = request.presetTitle;
        response(new IPC.ShellSaveResponse());
    }

    private _ipc_ShellPwdRequest(
        request: IPC.ShellPwdRequest,
        response: (response: IPC.ShellPwdResponse) => Promise<void>,
    ) {
        if (request.session !== this._guid) {
            return;
        }
        dialog
            .showOpenDialog({ properties: ['openDirectory'] })
            .then((value: OpenDialogReturnValue) => {
                const dirPath: string | undefined =
                    value.filePaths[0] === undefined ? '' : value.filePaths[0];
                this._env.pwd = dirPath;
                response(
                    new IPC.ShellPwdResponse({
                        value: dirPath,
                        guid: this._guid,
                    }),
                );
            })
            .catch((error: Error) => {
                response(
                    new IPC.ShellPwdResponse({
                        value: '',
                        guid: this._guid,
                        error: error.message,
                    }),
                );
            });
    }

    private _ipc_ShellRecentCommandsRemove(request: IPC.ShellRecentCommandsRemove) {
        if (request.session !== this._guid) {
            return;
        }
        ServiceStorage.get()
            .set({
                recentCommands: ServiceStorage.get()
                    .get()
                    .recentCommands.filter((command: string) => {
                        return command !== request.command;
                    }),
            })
            .catch((err: Error) => {
                this._logger.error(
                    `Failed to remove command from recent due error: ${err.message}`,
                );
            });
    }
}
