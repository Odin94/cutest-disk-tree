import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { debugLog, findFiles } from "../api"
import { Button } from "../components/ui/button"
import type { FileSearchResult, ScanProgress, ScanResult } from "../types"
import { humanSize } from "../utils"
import { FindResultsTable } from "./FileFindingView/FindResultsTable"

type TabId = "folders" | "find"
export type FindSortKey = "name" | "size" | "path"
export type FindSortDirection = "asc" | "desc"
type FileCategory = "all" | "audio" | "document" | "video" | "image" | "executable" | "compressed" | "config" | "folder" | "other"

export const MAX_VISIBLE_FILES = 500

type FileFindingViewProps = {
    result: ScanResult | null
    loading: boolean
    error: string | null
    progress: ScanProgress | null
    onScan: () => void
    onCancelScan?: () => void
}

export const getFileName = (path: string): string => {
    const segments = path.split(/[/\\]/)
    if (segments.length === 0) return path
    return segments[segments.length - 1] ?? path
}

export const FileFindingView = ({ result, loading, error, progress, onScan, onCancelScan }: FileFindingViewProps) => {
    const renderT0 = performance.now()
    const [activeTab, setActiveTab] = useState<TabId>("find")
    const [searchQuery, setSearchQuery] = useState("")
    const [searchExtensions, setSearchExtensions] = useState("")
    const [searchCategory, setSearchCategory] = useState<FileCategory>("all")
    const [useFuzzySearch, setUseFuzzySearch] = useState(false)
    const [searchResults, setSearchResults] = useState<FileSearchResult[]>([])
    const [searchTotalCount, setSearchTotalCount] = useState<number | null>(null)
    const [searchNextOffset, setSearchNextOffset] = useState<number | null>(null)
    const [searchLoadingMore, setSearchLoadingMore] = useState(false)
    const [searchLoading, setSearchLoading] = useState(false)
    const [searchError, setSearchError] = useState<string | null>(null)
    const [findSortKey, setFindSortKey] = useState<FindSortKey>("name")
    const [findSortDirection, setFindSortDirection] = useState<FindSortDirection>("asc")
    const lastQueryRef = useRef<{
        query: string
        extensions: string
        category: FileCategory
        fuzzy: boolean
    } | null>(null)

    useEffect(() => {
        lastQueryRef.current = null
    }, [result])

    const { filesCount, folderList } = useMemo(() => {
        if (result === null) {
            return {
                filesCount: 0,
                folderList: [] as { path: string; size: number }[],
            }
        }

        const t0 = performance.now()

        const filesCount = result.files_count ?? result.files.length

        const folders = Object.entries(result.folder_sizes)
            .map(([path, size]) => ({ path, size }))
            .sort((a, b) => b.size - a.size)

        const ms = (performance.now() - t0).toFixed(1)
        debugLog(`find_ui summary_compute ms=${ms} files_count=${filesCount} folders=${folders.length}`)

        return {
            filesCount,
            folderList: folders,
        }
    }, [result])

    const canSearch = result !== null && !loading

    const getEffectiveExtensions = (): string => {
        const manual =
            searchExtensions.trim().length === 0
                ? []
                : searchExtensions
                      .split(",")
                      .map((raw) => raw.trim().replace(/^\./, ""))
                      .filter((raw) => raw.length > 0)
        if (manual.length === 0) return ""
        const unique = Array.from(new Set(manual.map((ext) => ext.toLowerCase())))
        return unique.join(", ")
    }

    const PAGE_SIZE = MAX_VISIBLE_FILES

    const runSearch = async (event: React.FormEvent) => {
        event.preventDefault()
        if (!canSearch || result === null) return
        const extensions = getEffectiveExtensions()
        const queryKey = {
            query: searchQuery,
            extensions,
            category: searchCategory,
            fuzzy: useFuzzySearch,
        }
        if (
            lastQueryRef.current &&
            lastQueryRef.current.query === queryKey.query &&
            lastQueryRef.current.extensions === queryKey.extensions &&
            lastQueryRef.current.category === queryKey.category &&
            lastQueryRef.current.fuzzy === queryKey.fuzzy
        ) {
            debugLog("FileFindingView runSearch skip (same query as current data)")
            return
        }
        const reqT0 = performance.now()
        debugLog(
            `find_ui find_files request start query_len=${searchQuery.length} ext=${extensions.length > 0 ? "set" : "no"} category=${searchCategory} fuzzy=${useFuzzySearch ? "on" : "off"}`,
        )
        setSearchLoading(true)
        setSearchLoadingMore(false)
        setSearchResults([])
        setSearchNextOffset(null)
        setSearchTotalCount(null)
        setSearchError(null)
        try {
            const response = await findFiles(searchQuery, extensions, searchCategory, useFuzzySearch, PAGE_SIZE, 0)
            lastQueryRef.current = queryKey
            const elapsed = (performance.now() - reqT0).toFixed(0)
            debugLog(
                `find_ui find_files response received count=${response.items.length} nextOffset=${response.nextOffset ?? "null"} elapsedMs=${elapsed}`,
            )
            setSearchResults(response.items)
            setSearchNextOffset(response.nextOffset)
        } catch (e) {
            debugLog(`FileFindingView runSearch error: ${e instanceof Error ? e.message : String(e)}`)
            setSearchError(e instanceof Error ? e.message : String(e))
        } finally {
            setSearchLoading(false)
        }
    }

    useEffect(() => {
        if (!canSearch || result === null || activeTab !== "find") return
        const timeoutId = window.setTimeout(() => {
            const extensions = getEffectiveExtensions()
            const queryKey = {
                query: searchQuery,
                extensions,
                category: searchCategory,
                fuzzy: useFuzzySearch,
            }
            if (
                lastQueryRef.current &&
                lastQueryRef.current.query === queryKey.query &&
                lastQueryRef.current.extensions === queryKey.extensions &&
                lastQueryRef.current.category === queryKey.category &&
                lastQueryRef.current.fuzzy === queryKey.fuzzy
            ) {
                debugLog("FileFindingView findFiles (debounced) skip (same query as current data)")
                return
            }
            const reqT0 = performance.now()
            debugLog(
                `find_ui find_files request start (debounced) query_len=${searchQuery.length} ext=${extensions.length > 0 ? "set" : "no"} category=${searchCategory} fuzzy=${useFuzzySearch ? "on" : "off"}`,
            )
            setSearchLoading(true)
            setSearchLoadingMore(false)
            setSearchResults([])
            setSearchNextOffset(null)
            setSearchTotalCount(null)
            setSearchError(null)
            findFiles(searchQuery, extensions, searchCategory, useFuzzySearch, PAGE_SIZE, 0)
                .then((response) => {
                    lastQueryRef.current = queryKey
                    const elapsed = (performance.now() - reqT0).toFixed(0)
                    debugLog(
                        `find_ui find_files response received (debounced) count=${response.items.length} nextOffset=${response.nextOffset ?? "null"} elapsedMs=${elapsed}`,
                    )
                    setSearchResults(response.items)
                    setSearchNextOffset(response.nextOffset)
                })
                .catch((e) => {
                    const msg = e instanceof Error ? e.message : String(e)
                    debugLog(`FileFindingView findFiles (debounced) error: ${msg}`)
                    setSearchError(msg)
                })
                .finally(() => setSearchLoading(false))
        }, 200)
        return () => window.clearTimeout(timeoutId)
    }, [activeTab, canSearch, searchQuery, searchExtensions, searchCategory, useFuzzySearch])

    const { visibleSearchResults, sortedCount } = useMemo(() => {
        const sorted = [...searchResults].sort((a, b) => {
            if (findSortKey === "size") {
                if (a.size === b.size) return 0
                return findSortDirection === "asc" ? a.size - b.size : b.size - a.size
            }
            if (findSortKey === "path") {
                const cmp = a.path.localeCompare(b.path)
                return findSortDirection === "asc" ? cmp : -cmp
            }
            const nameA = getFileName(a.path)
            const nameB = getFileName(b.path)
            const cmp = nameA.localeCompare(nameB)
            return findSortDirection === "asc" ? cmp : -cmp
        })
        return {
            visibleSearchResults: sorted,
            sortedCount: sorted.length,
        }
    }, [searchResults, findSortKey, findSortDirection])

    const totalMatches = searchTotalCount != null ? searchTotalCount : sortedCount

    const canLoadMore = searchNextOffset != null

    const handleLoadMore = async () => {
        if (!canSearch || result === null) return
        if (!canLoadMore) return
        const reqT0 = performance.now()
        debugLog(
            `find_ui find_files request load_more query_len=${searchQuery.length} ext=${getEffectiveExtensions().length > 0 ? "set" : "no"} category=${searchCategory} offset=${searchNextOffset}`,
        )
        setSearchLoadingMore(true)
        try {
            const response = await findFiles(searchQuery, getEffectiveExtensions(), searchCategory, useFuzzySearch, PAGE_SIZE, searchNextOffset ?? 0)
            const elapsed = (performance.now() - reqT0).toFixed(0)
            debugLog(
                `find_ui find_files response received load_more count=${response.items.length} nextOffset=${response.nextOffset ?? "null"} elapsedMs=${elapsed}`,
            )
            setSearchResults((prev) => [...prev, ...response.items])
            setSearchNextOffset(response.nextOffset)
        } catch (e) {
            debugLog(`FileFindingView loadMore error: ${e instanceof Error ? e.message : String(e)}`)
            setSearchError(e instanceof Error ? e.message : String(e))
        } finally {
            setSearchLoadingMore(false)
        }
    }

    const toggleFindSort = useCallback((key: FindSortKey) => {
        setFindSortKey((current) => {
            if (current === key) {
                setFindSortDirection((d) => (d === "asc" ? "desc" : "asc"))
                return current
            }
            setFindSortDirection("asc")
            return key
        })
    }, [])

    const view = (
        <>
            <div className="file-finding-actions">
                <Button
                    type="button"
                    onClick={() => {
                        debugLog("FileFindingView click Scan filesystem")
                        onScan()
                    }}
                    disabled={loading}
                >
                    {loading ? "Scanning…" : "Scan filesystem"}
                </Button>
            </div>

            {error != null ? <div className="error">{error}</div> : null}

            {loading ? (
                <div className="progress-panel">
                    <div className="progress-bar" role="progressbar" aria-valuenow={progress?.files_count ?? 0} aria-label="Scanning files">
                        <div className="progress-bar-inner" />
                    </div>
                    <p className="progress-text">
                        {progress?.status != null
                            ? progress.status
                            : progress != null
                              ? `${progress.files_count.toLocaleString()} files scanned…`
                              : "Starting scan…"}
                    </p>
                    {progress != null ? (
                        <>
                            <p className="progress-text">{`${progress.files_count.toLocaleString()} files scanned`}</p>
                            {progress.current_path != null ? (
                                <p className="progress-path" title={progress.current_path}>
                                    {progress.current_path}
                                </p>
                            ) : null}
                        </>
                    ) : null}
                    {onCancelScan != null ? (
                        <p className="progress-actions">
                            <Button
                                type="button"
                                variant="secondary"
                                size="sm"
                                onClick={() => {
                                    debugLog("FileFindingView click Cancel scan")
                                    onCancelScan()
                                }}
                            >
                                Cancel scan
                            </Button>
                        </p>
                    ) : null}
                </div>
            ) : null}

            {result !== null && !loading ? (
                <>
                    <section className="summary">
                        <div className="summary-row">
                            <span className="label">Scanned roots:</span>
                            <span className="path">{result.roots.join(", ")}</span>
                        </div>
                        <div className="summary-row">
                            <span className="label">Files scanned:</span>
                            <span>{filesCount.toLocaleString()}</span>
                        </div>
                    </section>

                    <div className="tabs">
                        {(["find", "folders"] as const).map((tab) => (
                            <button
                                key={tab}
                                type="button"
                                className={activeTab === tab ? "tab active" : "tab"}
                                onClick={() => {
                                    const label = tab === "folders" ? "Largest folders" : "Find files"
                                    debugLog(`FileFindingView click tab ${tab} (${label})`)
                                    setActiveTab(tab)
                                }}
                            >
                                {tab === "folders" ? "Largest folders" : "Find files"}
                            </button>
                        ))}
                    </div>

                    <div className="panel">
                        {activeTab === "folders" ? (
                            <ul className="list folder-list">
                                {folderList.slice(0, 100).map(({ path, size }) => (
                                    <li key={path} className="list-item">
                                        <span className="size">{humanSize(size)}</span>
                                        <span className="path">{path}</span>
                                    </li>
                                ))}
                            </ul>
                        ) : (
                            <div className="find-panel">
                                <form className="find-form" onSubmit={runSearch}>
                                    <div className="find-fields">
                                        <label className="find-field">
                                            <span className="find-label">File name</span>
                                            <input
                                                type="text"
                                                value={searchQuery}
                                                onChange={(e) => {
                                                    const v = e.target.value
                                                    debugLog(`FileFindingView input file_name value=${JSON.stringify(v)}`)
                                                    setSearchQuery(v)
                                                }}
                                                placeholder="e.g. package, main.tsx"
                                                disabled={!canSearch}
                                            />
                                        </label>
                                        <label className="find-field">
                                            <span className="find-label">Category</span>
                                            <select
                                                value={searchCategory}
                                                onChange={(e) => {
                                                    const v = e.target.value as FileCategory
                                                    debugLog(`FileFindingView select category value=${v}`)
                                                    setSearchCategory(v)
                                                }}
                                                disabled={!canSearch}
                                            >
                                                <option value="all">All types</option>
                                                <option value="audio">Audio</option>
                                                <option value="document">Document</option>
                                                <option value="video">Video</option>
                                                <option value="image">Image</option>
                                                <option value="executable">Executable</option>
                                                <option value="compressed">Compressed</option>
                                                <option value="config">Config</option>
                                                <option value="folder">Folder</option>
                                                <option value="other">Other</option>
                                            </select>
                                        </label>
                                        <label className="find-field">
                                            <span className="find-label">File endings</span>
                                            <input
                                                type="text"
                                                value={searchExtensions}
                                                onChange={(e) => {
                                                    const v = e.target.value
                                                    debugLog(`FileFindingView input file_endings value=${JSON.stringify(v)}`)
                                                    setSearchExtensions(v)
                                                }}
                                                placeholder="e.g. ts, rs, .log"
                                                disabled={!canSearch}
                                            />
                                        </label>
                                        <label className="find-field">
                                            <span className="find-label">Fuzzy search</span>
                                            <input
                                                type="checkbox"
                                                checked={useFuzzySearch}
                                                onChange={(e) => {
                                                    const checked = e.target.checked
                                                    debugLog(`FileFindingView toggle fuzzy_search value=${JSON.stringify(checked)}`)
                                                    setUseFuzzySearch(checked)
                                                }}
                                                disabled={!canSearch}
                                            />
                                        </label>
                                        <Button
                                            type="submit"
                                            variant="secondary"
                                            size="sm"
                                            disabled={!canSearch || searchLoading}
                                            onClick={() => debugLog("FileFindingView click Find files submit")}
                                        >
                                            {searchLoading ? "Searching…" : "Find files"}
                                        </Button>
                                    </div>
                                    <p className="find-help">
                                        Uses fast substring matching by default; enable fuzzy search to use nucleo-based fuzzy matching
                                        against the indexed files.
                                    </p>
                                </form>
                                {searchError !== null ? <div className="error find-error">{searchError}</div> : null}
                                {searchLoading ? (
                                    <p className="find-status">Searching files…</p>
                                ) : totalMatches > 0 ? (
                                    <p className="find-status">
                                        {`${totalMatches.toLocaleString()} matching item${
                                            totalMatches === 1 ? "" : "s"
                                        }${canLoadMore ? " (more available…)" : ""}`}
                                    </p>
                                ) : null}
                                <FindResultsTable
                                    visibleSearchResults={visibleSearchResults}
                                    searchLoading={searchLoading}
                                    findSortKey={findSortKey}
                                    findSortDirection={findSortDirection}
                                    onToggleSort={toggleFindSort}
                                    hasMore={canLoadMore}
                                    loadingMore={searchLoadingMore}
                                    onLoadMore={handleLoadMore}
                                />
                            </div>
                        )}
                    </div>
                </>
            ) : loading ? null : (
                <div className="empty">Click &quot;Scan filesystem&quot; to start.</div>
            )}
        </>
    )

    const renderMs = performance.now() - renderT0
    if (renderMs > 10) {
        debugLog(
            `find_ui FileFindingView render slow ms=${renderMs.toFixed(1)} query_len=${searchQuery.length} results=${searchResults.length} loading=${searchLoading}`,
        )
    }

    return view
}
