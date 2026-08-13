using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Threading;
using System.Web.Script.Serialization;
using Autodesk.AutoCAD.ApplicationServices;
using Autodesk.AutoCAD.DatabaseServices;
using Autodesk.AutoCAD.Geometry;
using Autodesk.AutoCAD.Runtime;
using AcApp = Autodesk.AutoCAD.ApplicationServices.Core.Application;

[assembly: ExtensionApplication(typeof(CADEggBridge.BridgeEntry))]
[assembly: CommandClass(typeof(CADEggBridge.BridgeEntry))]

namespace CADEggBridge
{
    public sealed class BridgeEntry : IExtensionApplication
    {
        private const int BridgePort = 50471;
        private const string BridgeVersion = "0.2.9.0";

        private static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
        private static TcpListener _listener;
        private static Thread _listenerThread;
        private static volatile bool _running;

        public void Initialize()
        {
            Log("Initialize called");
            StartListener();
        }

        public void Terminate()
        {
            Log("Terminate called");
            StopListener();
        }

        [CommandMethod("CADEGGBRIDGESTATUS")]
        public void ShowStatus()
        {
            var doc = AcApp.DocumentManager.MdiActiveDocument;
            if (doc == null)
            {
                return;
            }

            doc.Editor.WriteMessage(
                "\nCADEgg bridge active. Port={0}, Version={1}",
                BridgePort,
                BridgeVersion
            );
        }

        private static void StartListener()
        {
            if (_running)
            {
                Log("StartListener skipped because bridge is already running");
                return;
            }

            try
            {
                _listener = new TcpListener(IPAddress.Loopback, BridgePort);
                _listener.Start(5);
                _running = true;
                _listenerThread = new Thread(ListenerLoop);
                _listenerThread.IsBackground = true;
                _listenerThread.Name = "CADEggBridgeListener";
                _listenerThread.Start();
                Log("Bridge listener started on 127.0.0.1:" + BridgePort);
            }
            catch (System.Exception ex)
            {
                _running = false;
                _listener = null;
                Log("StartListener failed: " + ex);
            }
        }

        private static void StopListener()
        {
            _running = false;
            try
            {
                if (_listener != null)
                {
                    _listener.Stop();
                }
            }
            catch (System.Exception ex)
            {
                Log("StopListener failed: " + ex);
            }
        }

        private static void ListenerLoop()
        {
            while (_running)
            {
                try
                {
                    var client = _listener.AcceptTcpClient();
                    ThreadPool.QueueUserWorkItem(HandleClient, client);
                }
                catch (SocketException)
                {
                    if (!_running)
                    {
                        Log("ListenerLoop stopped after listener shutdown");
                        return;
                    }
                }
                catch (System.Exception ex)
                {
                    Log("ListenerLoop error: " + ex);
                }
            }
        }

        private static void HandleClient(object state)
        {
            var client = state as TcpClient;
            if (client == null)
            {
                return;
            }

            using (client)
            using (var stream = client.GetStream())
            using (var reader = new StreamReader(stream))
            using (var writer = new StreamWriter(stream))
            {
                writer.AutoFlush = true;

                try
                {
                    var line = reader.ReadLine();
                    if (string.IsNullOrWhiteSpace(line))
                    {
                        writer.WriteLine(Json.Serialize(new BridgeResponse
                        {
                            ok = false,
                            message = "empty request",
                            data = new Dictionary<string, object>()
                        }));
                        return;
                    }

                    var request = Json.Deserialize<BridgeRequest>(line);
                    Log("Received command: " + (request.command ?? string.Empty));
                    var response = ExecuteRequest(request);
                    writer.WriteLine(Json.Serialize(response));
                }
                catch (System.Exception ex)
                {
                    Log("HandleClient error: " + ex);
                    writer.WriteLine(Json.Serialize(new BridgeResponse
                    {
                        ok = false,
                        message = ex.Message,
                        data = new Dictionary<string, object>()
                    }));
                }
            }
        }

        private static BridgeResponse ExecuteRequest(BridgeRequest request)
        {
            try
            {
                var data = Dispatch(request);
                var response = new BridgeResponse
                {
                    ok = true,
                    message = "ok",
                    data = data
                };
                Log("Command completed: " + (request.command ?? string.Empty));
                return response;
            }
            catch (System.Exception ex)
            {
                Log("Command failed: " + (request.command ?? string.Empty) + " :: " + ex);
                return new BridgeResponse
                {
                    ok = false,
                    message = ex.Message,
                    data = new Dictionary<string, object>()
                };
            }
        }

        private static Dictionary<string, object> Dispatch(BridgeRequest request)
        {
            var command = (request.command ?? string.Empty).Trim().ToLowerInvariant();
            switch (command)
            {
                case "ping":
                    return Ping();
                case "draw_line":
                    return DrawLine(request.args);
                case "draw_circle":
                    return DrawCircle(request.args);
                case "inspect_handle":
                    return InspectHandle(request.args);
                case "erase_handle":
                    return EraseHandle(request.args);
                case "mirror_handle":
                    return MirrorHandle(request.args);
                case "offset_handle":
                    return OffsetHandle(request.args);
                case "trim_by_handle":
                    return TrimByHandle(request.args);
                case "extend_by_handle":
                    return ExtendByHandle(request.args);
                default:
                    throw new InvalidOperationException("unsupported bridge command: " + request.command);
            }
        }

        private static Dictionary<string, object> Ping()
        {
            var doc = AcApp.DocumentManager.MdiActiveDocument;
            return new Dictionary<string, object>
            {
                { "bridge_version", BridgeVersion },
                { "document_name", doc != null ? doc.Name : string.Empty },
                { "acad_version", Convert.ToString(AcApp.GetSystemVariable("ACADVER")) ?? string.Empty }
            };
        }

        private static Dictionary<string, object> DrawLine(Dictionary<string, object> args)
        {
            var doc = AcApp.DocumentManager.MdiActiveDocument;
            if (doc == null)
            {
                throw new InvalidOperationException("no active AutoCAD document");
            }

            var x1 = ToDouble(args, "x1");
            var y1 = ToDouble(args, "y1");
            var x2 = ToDouble(args, "x2");
            var y2 = ToDouble(args, "y2");

            using (doc.LockDocument())
            {
                var db = doc.Database;
                using (var tr = db.TransactionManager.StartTransaction())
                {
                    var blockTable = (BlockTable)tr.GetObject(db.BlockTableId, OpenMode.ForRead);
                    var modelSpace = (BlockTableRecord)tr.GetObject(
                        blockTable[BlockTableRecord.ModelSpace],
                        OpenMode.ForWrite
                    );

                    var line = new Line(new Point3d(x1, y1, 0.0), new Point3d(x2, y2, 0.0));
                    modelSpace.AppendEntity(line);
                    tr.AddNewlyCreatedDBObject(line, true);
                    var summary = SummarizeObject(line, "LINE");
                    tr.Commit();
                    return summary;
                }
            }
        }

        private static Dictionary<string, object> DrawCircle(Dictionary<string, object> args)
        {
            var doc = AcApp.DocumentManager.MdiActiveDocument;
            if (doc == null)
            {
                throw new InvalidOperationException("no active AutoCAD document");
            }

            var cx = ToDouble(args, "cx");
            var cy = ToDouble(args, "cy");
            var radius = ToDouble(args, "r");
            if (radius <= 0.0)
            {
                throw new InvalidOperationException("radius must be positive");
            }

            using (doc.LockDocument())
            {
                var db = doc.Database;
                using (var tr = db.TransactionManager.StartTransaction())
                {
                    var blockTable = (BlockTable)tr.GetObject(db.BlockTableId, OpenMode.ForRead);
                    var modelSpace = (BlockTableRecord)tr.GetObject(
                        blockTable[BlockTableRecord.ModelSpace],
                        OpenMode.ForWrite
                    );

                    var circle = new Circle(new Point3d(cx, cy, 0.0), Vector3d.ZAxis, radius);
                    modelSpace.AppendEntity(circle);
                    tr.AddNewlyCreatedDBObject(circle, true);
                    var summary = SummarizeObject(circle, "CIRCLE");
                    tr.Commit();
                    return summary;
                }
            }
        }

        private static Dictionary<string, object> InspectHandle(Dictionary<string, object> args)
        {
            var doc = AcApp.DocumentManager.MdiActiveDocument;
            if (doc == null)
            {
                throw new InvalidOperationException("no active AutoCAD document");
            }

            var handleText = ToRequiredString(args, "handle");
            using (doc.LockDocument())
            {
                var db = doc.Database;
                using (var tr = db.TransactionManager.StartTransaction())
                {
                    var objectId = GetObjectIdByHandle(db, handleText);
                    var dbObject = tr.GetObject(objectId, OpenMode.ForRead);
                    return SummarizeObject(dbObject, GetObjectKind(dbObject));
                }
            }
        }

        private static Dictionary<string, object> EraseHandle(Dictionary<string, object> args)
        {
            var doc = AcApp.DocumentManager.MdiActiveDocument;
            if (doc == null)
            {
                throw new InvalidOperationException("no active AutoCAD document");
            }

            var handleText = ToRequiredString(args, "handle");
            using (doc.LockDocument())
            {
                var db = doc.Database;
                using (var tr = db.TransactionManager.StartTransaction())
                {
                    var objectId = GetObjectIdByHandle(db, handleText);
                    var dbObject = tr.GetObject(objectId, OpenMode.ForWrite);
                    var summary = SummarizeObject(dbObject, GetObjectKind(dbObject));
                    dbObject.Erase();
                    tr.Commit();
                    return summary;
                }
            }
        }

        private static Dictionary<string, object> MirrorHandle(Dictionary<string, object> args)
        {
            var doc = AcApp.DocumentManager.MdiActiveDocument;
            if (doc == null)
            {
                throw new InvalidOperationException("no active AutoCAD document");
            }

            var handleText = ToRequiredString(args, "handle");
            var x1 = ToDouble(args, "x1");
            var y1 = ToDouble(args, "y1");
            var x2 = ToDouble(args, "x2");
            var y2 = ToDouble(args, "y2");
            if (x1 == x2 && y1 == y2)
            {
                throw new InvalidOperationException("mirror axis points must differ");
            }

            using (doc.LockDocument())
            {
                var db = doc.Database;
                using (var tr = db.TransactionManager.StartTransaction())
                {
                    var objectId = GetObjectIdByHandle(db, handleText);
                    var source = tr.GetObject(objectId, OpenMode.ForRead) as Entity;
                    if (source == null)
                    {
                        throw new InvalidOperationException("object is not an entity for handle=" + handleText);
                    }

                    var clone = source.Clone() as Entity;
                    if (clone == null)
                    {
                        throw new InvalidOperationException("failed to clone entity for handle=" + handleText);
                    }

                    var axis = new Line3d(new Point3d(x1, y1, 0.0), new Point3d(x2, y2, 0.0));
                    clone.TransformBy(Matrix3d.Mirroring(axis));

                    var blockTable = (BlockTable)tr.GetObject(db.BlockTableId, OpenMode.ForRead);
                    var modelSpace = (BlockTableRecord)tr.GetObject(
                        blockTable[BlockTableRecord.ModelSpace],
                        OpenMode.ForWrite
                    );
                    modelSpace.AppendEntity(clone);
                    tr.AddNewlyCreatedDBObject(clone, true);
                    var summary = SummarizeObject(clone, GetObjectKind(clone));
                    tr.Commit();
                    return summary;
                }
            }
        }

        private static Dictionary<string, object> OffsetHandle(Dictionary<string, object> args)
        {
            var doc = AcApp.DocumentManager.MdiActiveDocument;
            if (doc == null)
            {
                throw new InvalidOperationException("no active AutoCAD document");
            }

            var handleText = ToRequiredString(args, "handle");
            var distance = ToDouble(args, "distance");
            var sideX = ToDouble(args, "side_x");
            var sideY = ToDouble(args, "side_y");
            if (distance <= 0.0)
            {
                throw new InvalidOperationException("offset distance must be positive");
            }

            using (doc.LockDocument())
            {
                var db = doc.Database;
                using (var tr = db.TransactionManager.StartTransaction())
                {
                    var objectId = GetObjectIdByHandle(db, handleText);
                    var curve = tr.GetObject(objectId, OpenMode.ForRead) as Curve;
                    if (curve == null)
                    {
                        throw new InvalidOperationException("offset bridge currently supports curve entities only");
                    }

                    var sidePoint = new Point3d(sideX, sideY, 0.0);
                    Entity bestCandidate = null;
                    var bestScore = double.MaxValue;
                    foreach (var candidate in BuildOffsetCandidates(curve, distance))
                    {
                        var score = DistanceToEntity(candidate, sidePoint);
                        if (score < bestScore)
                        {
                            if (bestCandidate != null)
                            {
                                bestCandidate.Dispose();
                            }

                            bestCandidate = candidate;
                            bestScore = score;
                        }
                        else
                        {
                            candidate.Dispose();
                        }
                    }

                    if (bestCandidate == null)
                    {
                        throw new InvalidOperationException("offset produced no result for handle=" + handleText);
                    }

                    var blockTable = (BlockTable)tr.GetObject(db.BlockTableId, OpenMode.ForRead);
                    var modelSpace = (BlockTableRecord)tr.GetObject(
                        blockTable[BlockTableRecord.ModelSpace],
                        OpenMode.ForWrite
                    );
                    modelSpace.AppendEntity(bestCandidate);
                    tr.AddNewlyCreatedDBObject(bestCandidate, true);
                    var summary = SummarizeObject(bestCandidate, GetObjectKind(bestCandidate));
                    tr.Commit();
                    return summary;
                }
            }
        }

        private static Dictionary<string, object> TrimByHandle(Dictionary<string, object> args)
        {
            var doc = AcApp.DocumentManager.MdiActiveDocument;
            if (doc == null)
            {
                throw new InvalidOperationException("no active AutoCAD document");
            }

            var boundaryHandle = ToRequiredString(args, "boundary_handle");
            var targetHandle = ToRequiredString(args, "target_handle");
            if (string.Equals(boundaryHandle, targetHandle, StringComparison.OrdinalIgnoreCase))
            {
                throw new InvalidOperationException("boundary_handle and target_handle must differ");
            }

            var pickPoint = new Point3d(ToDouble(args, "pick_x"), ToDouble(args, "pick_y"), 0.0);
            Log("TrimByHandle start boundary=" + boundaryHandle + " target=" + targetHandle);
            using (doc.LockDocument())
            {
                var db = doc.Database;
                using (var tr = db.TransactionManager.StartTransaction())
                {
                    var boundary = tr.GetObject(GetObjectIdByHandle(db, boundaryHandle), OpenMode.ForRead) as Line;
                    if (boundary == null)
                    {
                        throw new InvalidOperationException("trim bridge currently supports LINE boundary only");
                    }

                    var target = tr.GetObject(GetObjectIdByHandle(db, targetHandle), OpenMode.ForWrite) as Line;
                    if (target == null)
                    {
                        throw new InvalidOperationException("trim bridge currently supports LINE target only");
                    }

                    var intersection = IntersectLineSegments(boundary, target, true);
                    var intersectionParam = ProjectLineParameter(target, intersection);
                    var pickParam = ProjectLineParameter(target, pickPoint);
                    var newStart = target.StartPoint;
                    var newEnd = target.EndPoint;
                    if (pickParam >= intersectionParam)
                    {
                        newEnd = intersection;
                    }
                    else
                    {
                        newStart = intersection;
                    }

                    var blockTable = (BlockTable)tr.GetObject(db.BlockTableId, OpenMode.ForRead);
                    var modelSpace = (BlockTableRecord)tr.GetObject(
                        blockTable[BlockTableRecord.ModelSpace],
                        OpenMode.ForWrite
                    );
                    var replacement = new Line(newStart, newEnd);
                    modelSpace.AppendEntity(replacement);
                    tr.AddNewlyCreatedDBObject(replacement, true);
                    target.Erase();

                    var summary = SummarizeObject(replacement, "LINE");
                    summary["replaced_handle"] = targetHandle;
                    tr.Commit();
                    Log("TrimByHandle committed replacement handle=" + Convert.ToString(summary["handle"], CultureInfo.InvariantCulture));
                    return summary;
                }
            }
        }

        private static Dictionary<string, object> ExtendByHandle(Dictionary<string, object> args)
        {
            var doc = AcApp.DocumentManager.MdiActiveDocument;
            if (doc == null)
            {
                throw new InvalidOperationException("no active AutoCAD document");
            }

            var boundaryHandle = ToRequiredString(args, "boundary_handle");
            var targetHandle = ToRequiredString(args, "target_handle");
            if (string.Equals(boundaryHandle, targetHandle, StringComparison.OrdinalIgnoreCase))
            {
                throw new InvalidOperationException("boundary_handle and target_handle must differ");
            }

            var pickPoint = new Point3d(ToDouble(args, "pick_x"), ToDouble(args, "pick_y"), 0.0);
            Log("ExtendByHandle start boundary=" + boundaryHandle + " target=" + targetHandle);
            using (doc.LockDocument())
            {
                var db = doc.Database;
                using (var tr = db.TransactionManager.StartTransaction())
                {
                    var boundary = tr.GetObject(GetObjectIdByHandle(db, boundaryHandle), OpenMode.ForRead) as Line;
                    if (boundary == null)
                    {
                        throw new InvalidOperationException("extend bridge currently supports LINE boundary only");
                    }

                    var target = tr.GetObject(GetObjectIdByHandle(db, targetHandle), OpenMode.ForWrite) as Line;
                    if (target == null)
                    {
                        throw new InvalidOperationException("extend bridge currently supports LINE target only");
                    }

                    var intersection = IntersectLineSegments(boundary, target, false);
                    var pickParam = ProjectLineParameter(target, pickPoint);
                    var newStart = target.StartPoint;
                    var newEnd = target.EndPoint;
                    if (pickParam >= 0.5)
                    {
                        newEnd = intersection;
                    }
                    else
                    {
                        newStart = intersection;
                    }

                    var blockTable = (BlockTable)tr.GetObject(db.BlockTableId, OpenMode.ForRead);
                    var modelSpace = (BlockTableRecord)tr.GetObject(
                        blockTable[BlockTableRecord.ModelSpace],
                        OpenMode.ForWrite
                    );
                    var replacement = new Line(newStart, newEnd);
                    modelSpace.AppendEntity(replacement);
                    tr.AddNewlyCreatedDBObject(replacement, true);
                    target.Erase();

                    var summary = SummarizeObject(replacement, "LINE");
                    summary["replaced_handle"] = targetHandle;
                    tr.Commit();
                    Log("ExtendByHandle committed replacement handle=" + Convert.ToString(summary["handle"], CultureInfo.InvariantCulture));
                    return summary;
                }
            }
        }

        private static double ToDouble(Dictionary<string, object> args, string key)
        {
            if (args == null || !args.ContainsKey(key))
            {
                throw new InvalidOperationException("missing bridge arg: " + key);
            }

            return Convert.ToDouble(args[key]);
        }

        private static string ToRequiredString(Dictionary<string, object> args, string key)
        {
            if (args == null || !args.ContainsKey(key))
            {
                throw new InvalidOperationException("missing bridge arg: " + key);
            }

            var value = Convert.ToString(args[key]) ?? string.Empty;
            value = value.Trim();
            if (value.Length == 0)
            {
                throw new InvalidOperationException("empty bridge arg: " + key);
            }

            return value;
        }

        private static List<Entity> BuildOffsetCandidates(Curve curve, double distance)
        {
            var candidates = new List<Entity>();
            AddOffsetCandidates(curve, distance, candidates);
            AddOffsetCandidates(curve, -distance, candidates);
            return candidates;
        }

        private static void AddOffsetCandidates(Curve curve, double distance, List<Entity> candidates)
        {
            try
            {
                var objects = curve.GetOffsetCurves(distance);
                foreach (DBObject obj in objects)
                {
                    var entity = obj as Entity;
                    if (entity != null)
                    {
                        candidates.Add(entity);
                    }
                    else if (obj != null)
                    {
                        obj.Dispose();
                    }
                }
            }
            catch
            {
            }
        }

        private static Point3d FindClosestIntersection(
            Entity source,
            Entity other,
            Intersect mode,
            Point3d reference
        )
        {
            var intersections = new Point3dCollection();
            source.IntersectWith(other, mode, intersections, IntPtr.Zero, IntPtr.Zero);
            if (intersections.Count == 0)
            {
                throw new InvalidOperationException("no intersection found for bridge editing operation");
            }

            var best = intersections[0];
            var bestDistance = best.DistanceTo(reference);
            for (var i = 1; i < intersections.Count; i++)
            {
                var candidate = intersections[i];
                var distance = candidate.DistanceTo(reference);
                if (distance < bestDistance)
                {
                    best = candidate;
                    bestDistance = distance;
                }
            }

            return best;
        }

        private static double ProjectLineParameter(Line line, Point3d point)
        {
            var dx = line.EndPoint.X - line.StartPoint.X;
            var dy = line.EndPoint.Y - line.StartPoint.Y;
            var dz = line.EndPoint.Z - line.StartPoint.Z;
            var denom = dx * dx + dy * dy + dz * dz;
            if (denom <= 1e-12)
            {
                return 0.0;
            }

            return (
                (point.X - line.StartPoint.X) * dx +
                (point.Y - line.StartPoint.Y) * dy +
                (point.Z - line.StartPoint.Z) * dz
            ) / denom;
        }

        private static Point3d IntersectLineSegments(Line boundary, Line target, bool requireTargetSegment)
        {
            var x1 = boundary.StartPoint.X;
            var y1 = boundary.StartPoint.Y;
            var x2 = boundary.EndPoint.X;
            var y2 = boundary.EndPoint.Y;
            var x3 = target.StartPoint.X;
            var y3 = target.StartPoint.Y;
            var x4 = target.EndPoint.X;
            var y4 = target.EndPoint.Y;
            var denominator = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4);
            if (Math.Abs(denominator) <= 1e-9)
            {
                throw new InvalidOperationException("boundary and target are parallel or coincident");
            }

            var detBoundary = x1 * y2 - y1 * x2;
            var detTarget = x3 * y4 - y3 * x4;
            var ix = (detBoundary * (x3 - x4) - (x1 - x2) * detTarget) / denominator;
            var iy = (detBoundary * (y3 - y4) - (y1 - y2) * detTarget) / denominator;
            var intersection = new Point3d(ix, iy, target.StartPoint.Z);
            if (!IsPointOnLineSegment(boundary, intersection))
            {
                throw new InvalidOperationException("intersection falls outside boundary segment");
            }

            if (requireTargetSegment && !IsPointOnLineSegment(target, intersection))
            {
                throw new InvalidOperationException("intersection falls outside target segment");
            }

            return intersection;
        }

        private static bool IsPointOnLineSegment(Line line, Point3d point)
        {
            var param = ProjectLineParameter(line, point);
            if (param < -1e-9 || param > 1.0 + 1e-9)
            {
                return false;
            }

            var projected = line.StartPoint + (line.EndPoint - line.StartPoint) * param;
            return projected.DistanceTo(point) <= 1e-6;
        }

        private static void SendStringToExecute(Document doc, string script, string opName)
        {
            Log(opName + " queue start");
            System.Exception error = null;
            using (var queued = new ManualResetEventSlim(false))
            {
                AcApp.DocumentManager.ExecuteInApplicationContext(
                    delegate(object state)
                    {
                        try
                        {
                            doc.SendStringToExecute(script, true, false, false);
                            Log(opName + " queued");
                        }
                        catch (System.Exception ex)
                        {
                            error = ex;
                            Log(opName + " queue failed: " + ex);
                        }
                        finally
                        {
                            queued.Set();
                        }
                    },
                    null
                );

                if (!queued.Wait(2000))
                {
                    throw new InvalidOperationException(opName + " queue timed out");
                }
            }

            if (error != null)
            {
                throw new InvalidOperationException(opName + " queue failed", error);
            }
        }

        private static double DistanceToEntity(Entity entity, Point3d point)
        {
            var curve = entity as Curve;
            if (curve != null)
            {
                return curve.GetClosestPointTo(point, false).DistanceTo(point);
            }

            var extents = entity.GeometricExtents;
            var center = new Point3d(
                (extents.MinPoint.X + extents.MaxPoint.X) / 2.0,
                (extents.MinPoint.Y + extents.MaxPoint.Y) / 2.0,
                (extents.MinPoint.Z + extents.MaxPoint.Z) / 2.0
            );
            return center.DistanceTo(point);
        }

        private static ObjectId GetObjectIdByHandle(Database db, string handleText)
        {
            long rawHandle;
            try
            {
                rawHandle = Convert.ToInt64(handleText, 16);
            }
            catch (System.Exception ex)
            {
                throw new InvalidOperationException("invalid handle: " + handleText, ex);
            }

            try
            {
                return db.GetObjectId(false, new Handle(rawHandle), 0);
            }
            catch (System.Exception ex)
            {
                throw new InvalidOperationException("object not found for handle=" + handleText, ex);
            }
        }

        private static Dictionary<string, object> SummarizeObject(DBObject dbObject, string fallbackKind)
        {
            var kind = GetObjectKind(dbObject);
            if (string.IsNullOrEmpty(kind))
            {
                kind = fallbackKind;
            }

            return new Dictionary<string, object>
            {
                { "handle", dbObject.Handle.ToString() },
                { "kind", kind },
                { "label", DescribeObject(dbObject, kind, fallbackKind) }
            };
        }

        private static string GetObjectKind(DBObject dbObject)
        {
            var rx = dbObject != null ? dbObject.GetRXClass() : null;
            var dxfName = rx != null ? rx.DxfName : null;
            if (string.IsNullOrWhiteSpace(dxfName))
            {
                return "UNKNOWN";
            }

            return dxfName.Trim().ToUpperInvariant();
        }

        private static string DescribeObject(DBObject dbObject, string kind, string fallbackLabel)
        {
            var line = dbObject as Line;
            if (line != null)
            {
                return string.Format(
                    CultureInfo.InvariantCulture,
                    "直线 ({0},{1}) → ({2},{3})",
                    FormatNumber(line.StartPoint.X),
                    FormatNumber(line.StartPoint.Y),
                    FormatNumber(line.EndPoint.X),
                    FormatNumber(line.EndPoint.Y)
                );
            }

            var circle = dbObject as Circle;
            if (circle != null)
            {
                return string.Format(
                    CultureInfo.InvariantCulture,
                    "圆心 ({0},{1}) 半径 {2}",
                    FormatNumber(circle.Center.X),
                    FormatNumber(circle.Center.Y),
                    FormatNumber(circle.Radius)
                );
            }

            var arc = dbObject as Arc;
            if (arc != null)
            {
                return string.Format(
                    CultureInfo.InvariantCulture,
                    "圆弧 圆心 ({0},{1}) 半径 {2} 角度 {3}°→{4}°",
                    FormatNumber(arc.Center.X),
                    FormatNumber(arc.Center.Y),
                    FormatNumber(arc.Radius),
                    FormatAngleDeg(arc.StartAngle),
                    FormatAngleDeg(arc.EndAngle)
                );
            }

            var polyline = dbObject as Polyline;
            if (polyline != null && polyline.NumberOfVertices > 0)
            {
                var preview = new List<string>();
                var limit = Math.Min(polyline.NumberOfVertices, 4);
                for (var i = 0; i < limit; i++)
                {
                    var point = polyline.GetPoint2dAt(i);
                    preview.Add(string.Format(
                        CultureInfo.InvariantCulture,
                        "({0},{1})",
                        FormatNumber(point.X),
                        FormatNumber(point.Y)
                    ));
                }

                var suffix = polyline.NumberOfVertices > 4 ? " → ..." : string.Empty;
                var closed = polyline.Closed ? " 闭合" : string.Empty;
                return string.Format(
                    CultureInfo.InvariantCulture,
                    "多段线 {0} 点{1}: {2}{3}",
                    polyline.NumberOfVertices,
                    closed,
                    string.Join(" → ", preview.ToArray()),
                    suffix
                );
            }

            var dbText = dbObject as DBText;
            if (dbText != null)
            {
                return string.Format(
                    CultureInfo.InvariantCulture,
                    "文字 \"{0}\" @ ({1},{2})",
                    dbText.TextString,
                    FormatNumber(dbText.Position.X),
                    FormatNumber(dbText.Position.Y)
                );
            }

            var mText = dbObject as MText;
            if (mText != null)
            {
                return string.Format(
                    CultureInfo.InvariantCulture,
                    "文字 \"{0}\" @ ({1},{2})",
                    mText.Contents,
                    FormatNumber(mText.Location.X),
                    FormatNumber(mText.Location.Y)
                );
            }

            return !string.IsNullOrEmpty(kind) ? kind : fallbackLabel;
        }

        private static string FormatAngleDeg(double radians)
        {
            return FormatNumber(radians * 180.0 / Math.PI);
        }

        private static string FormatNumber(double value)
        {
            if (Math.Abs(value % 1.0) < 1e-9)
            {
                return value.ToString("F1", CultureInfo.InvariantCulture);
            }

            return value.ToString("0.###############", CultureInfo.InvariantCulture);
        }

        private static void Log(string message)
        {
            try
            {
                var dir = Path.Combine(
                    Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                    "CADEgg"
                );
                Directory.CreateDirectory(dir);
                var path = Path.Combine(dir, "bridge.log");
                File.AppendAllText(
                    path,
                    string.Format(
                        CultureInfo.InvariantCulture,
                        "[{0}] {1}{2}",
                        DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss.fff", CultureInfo.InvariantCulture),
                        message,
                        Environment.NewLine
                    )
                );
            }
            catch
            {
            }
        }
    }

    public sealed class BridgeRequest
    {
        public string command;
        public Dictionary<string, object> args;
    }

    public sealed class BridgeResponse
    {
        public bool ok;
        public string message;
        public Dictionary<string, object> data;
    }
}
